package tailcall.runtime.transcoder

import caliban.InputValue
import tailcall.runtime.JsonT
import tailcall.runtime.http.HttpClient
import tailcall.runtime.lambda.Syntax._
import tailcall.runtime.lambda._
import tailcall.runtime.model.Config.{Arg, Field, Type}
import tailcall.runtime.model.UnsafeSteps.Operation
import tailcall.runtime.model.{Config, Path}
import tailcall.runtime.service._
import zio.http.model.Headers
import zio.http.{Request, URL => ZURL}
import zio.json.ast.Json
import zio.test.Assertion.equalTo
import zio.test.TestAspect.{before, parallel, timeout}
import zio.test.{TestConsole, TestSystem, ZIOSpecDefault, assertTrue, assertZIO}
import zio.{Chunk, ZIO, durationInt}

import java.net.URI

/**
 * Tests for the generation of GraphQL steps from a config.
 * This is done by writing a test config, converting to
 * graphql and testing it with sample graphql queries.
 */
object Config2StepSpec extends ZIOSpecDefault {
  override def spec =
    suite("Config to GraphQL Step")(
      test("rename a field") {
        val config  = {
          Config.default
            .withTypes("Query" -> Type("foo" -> Field.ofType("String").resolveWithJson("Hello World!").withName("bar")))
        }
        val program = resolve(config)(""" query { bar } """)

        assertZIO(program)(equalTo("""{"bar":"Hello World!"}"""))
      },
      test("rename an argument") {
        val config  = {
          Config.default.withTypes(
            "Query" -> Type(
              "foo" -> Field.ofType("Bar").withArguments("input" -> Arg.ofType("Int").withName("data"))
                .withSteps(Operation.objPath("bar" -> List("args", "data")))
            ),
            "Bar"   -> Type("bar" -> Field.ofType("Int")),
          )
        }
        val program = resolve(config)(""" query { foo(data: 1) {bar} } """)

        assertZIO(program)(equalTo("""{"foo":{"bar":1}}"""))
      },
      test("nested type") {
        val value = Json.Obj(
          "b" -> Json.Arr(Json.Obj("c" -> Json.Num(1)), Json.Obj("c" -> Json.Num(2)), Json.Obj("c" -> Json.Num(3)))
        )

        val config = Config.default.withTypes(
          "Query" -> Type("a" -> Field.ofType("A").withSteps(Operation.constant(value))),
          "A"     -> Type("b" -> Field.ofType("B").asList),
          "B"     -> Type("c" -> Field.int),
        )

        val program = resolve(config)("""{a {b {c}}}""")
        assertZIO(program)(equalTo("""{"a":{"b":[{"c":1},{"c":2},{"c":3}]}}"""))
      },
      test("dictionary") {
        val value: Json = Json
          .Obj("a" -> Json.Num(1), "b" -> Json.Obj("k1" -> Json.Num(1), "k2" -> Json.Num(2), "k3" -> Json.Num(3)))

        val transformation = JsonT.applySpec("a" -> JsonT.path("a"), "b" -> JsonT.path("b").pipe(JsonT.toKeyValue))

        val config = Config.default.withTypes(
          "Query" -> Type(
            "z" -> Field.ofType("A").withSteps(Operation.constant(value), Operation.transform(transformation))
          ),
          "A"     -> Type("a" -> Field.int, "b" -> Field.ofType("B").asList),
          "B"     -> Type("key" -> Field.string, "value" -> Field.int),
        )

        val program = resolve(config)("""{z {a b {key value}}}""")
        assertZIO(program)(equalTo(
          """{"z":{"a":1,"b":[{"key":"k1","value":1},{"key":"k2","value":2},{"key":"k3","value":3}]}}"""
        ))
      },
      test("simple query") {
        val config  = Config.default
          .withTypes("Query" -> Type("foo" -> Field.ofType("String").resolveWithJson("Hello World!")))
        val program = resolve(config)(" {foo} ")
        assertZIO(program)(equalTo("""{"foo":"Hello World!"}"""))
      },
      test("nested objects") {
        val config = Config.default.withTypes(
          "Query" -> Type("foo" -> Field.ofType("Foo").resolveWithJson(Map("bar" -> "Hello World!"))),
          "Foo"   -> Type("bar" -> Field.ofType("String")),
        )

        val program = resolve(config)(" {foo {bar}} ")
        assertZIO(program)(equalTo("""{"foo":{"bar":"Hello World!"}}"""))
      },
      test("static value") {
        val config  = Config.default
          .withTypes("Query" -> Config.Type("id" -> Config.Field.ofType("String").resolveWith(100)))
        val program = resolve(config)("query {id}")
        assertZIO(program)(equalTo("""{"id":100}"""))
      },
      test("with args") {
        val config  = Config.default.withTypes(
          "Query" -> Config.Type(
            "sum" -> Config.Field.ofType("Int")
              .withArguments("a" -> Config.Arg.ofType("Int"), "b" -> Config.Arg.ofType("Int"))
              .resolveWithFunction { ctx =>
                {
                  (for {
                    a <- ctx.toTypedPath[Int]("args", "a")
                    b <- ctx.toTypedPath[Int]("args", "b")
                  } yield a + b).toDynamic
                }
              }
          )
        )
        val program = resolve(config)("query {sum(a: 1, b: 2)}")
        assertZIO(program)(equalTo("""{"sum":3}"""))
      },
      test("with nesting") {
        val config = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo")),
          "Foo"   -> Config.Type("bar" -> Config.Field.ofType("Bar")),
          "Bar"   -> Config.Type("value" -> Config.Field.ofType("Int").resolveWith(100)),
        )

        val program = resolve(config)("query {foo { bar { value }}}")
        assertZIO(program)(equalTo("{\"foo\":{\"bar\":{\"value\":100}}}"))
      },
      test("with nesting array") {
        val config = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo")),
          "Foo"   -> Config.Type("bar" -> Config.Field.ofType("Bar").asList.resolveWith(List(100, 200, 300))),
          "Bar"   -> Config.Type("value" -> Config.Field.ofType("Int").resolveWith(100)),
        )

        val program = resolve(config)("query {foo { bar { value }}}")
        assertZIO(program)(equalTo("""{"foo":{"bar":[{"value":100},{"value":100},{"value":100}]}}"""))
      },
      test("with nesting array ctx") {
        val config = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo")),
          "Foo"   -> Config.Type("bar" -> Config.Field.ofType("Bar").asList.resolveWith(List(100, 200, 300))),
          "Bar"   -> Config.Type("value" -> Config.Field.ofType("Int").resolveWithFunction {
            _.toTypedPath[Int]("value").map(_ + Lambda(1)).toDynamic
          }),
        )

        val program = resolve(config)("query {foo { bar { value }}}")
        assertZIO(program)(equalTo("""{"foo":{"bar":[{"value":101},{"value":201},{"value":301}]}}"""))
      },
      test("with nesting level 3") {
        val config = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo")),
          "Foo"   -> Config.Type("bar" -> Config.Field.ofType("Bar").asList.resolveWith(List(100, 200, 300))),
          "Bar"   -> Config.Type("baz" -> Config.Field.ofType("Baz").resolveWithFunction {
            _.toTypedPath[Int]("value").map(_ + Lambda(1)).toDynamic
          }),
          "Baz"   -> Config.Type("value" -> Config.Field.ofType("Int").resolveWithFunction {
            _.toTypedPath[Option[Int]]("value").flatten.map(_ + Lambda(1)).toDynamic
          }),
        )

        val program = resolve(config)("query {foo { bar { baz {value} }}}")
        assertZIO(program)(equalTo(
          """{"foo":{"bar":[{"baz":{"value":102}},{"baz":{"value":202}},{"baz":{"value":302}}]}}"""
        ))
      },
      test("partial resolver") {
        val config  = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo").resolveWith(Map("a" -> 1, "b" -> 2))),
          "Foo"   -> Config.Type(
            "a" -> Config.Field.ofType("Int"),
            "b" -> Config.Field.ofType("Int"),
            "c" -> Config.Field.ofType("Int").resolveWith(3),
          ),
        )
        val program = resolve(config)("query {foo { a b c }}")
        assertZIO(program)(equalTo("""{"foo":{"a":1,"b":2,"c":3}}"""))

      },
      test("default property resolver") {
        val config  = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo").resolveWith(Map("a" -> 1))),
          "Foo"   -> Config.Type("a" -> Config.Field.ofType("Int")),
        )
        val program = resolve(config)("query {foo { a }}")
        assertZIO(program)(equalTo("""{"foo":{"a":1}}"""))

      },
      test("mutation with input type") {
        val config = Config.default.withMutation("Mutation").withTypes(
          "Query"    -> Config.Type("foo" -> Config.Field.ofType("Foo")),
          "Mutation" -> Config.Type(
            "createFoo" -> Config.Field.ofType("Foo").withArguments("input" -> Config.Arg.ofType("FooInput"))
              .resolveWith(Map("a" -> 1))
          ),
          "Foo"      -> Config.Type("a" -> Config.Field.ofType("Int")),
          "FooInput" -> Config.Type(
            "a" -> Config.Field.ofType("Int"),
            "b" -> Config.Field.ofType("Int"),
            "c" -> Config.Field.ofType("Int"),
          ),
        )

        val program = resolve(config, Map.empty)("mutation {createFoo(input: {a: 1}){a}}")
        assertZIO(program)(equalTo("""{"createFoo":{"a":1}}"""))
      },
      test("Query with list fields") {
        val json   = Json.Obj("a" -> Json.Num(1))
        val config = Config.default.withTypes(
          "Query" -> Config.Type("foo" -> Config.Field.ofType("Foo").resolveWithJson(json)),
          "Foo"   -> Config.Type("a" -> Config.Field.ofType("Int"), "b" -> Config.Field.ofType("Int").asList),
        )

        val program = resolve(config)("query {foo {a b}}")
        assertZIO(program)(equalTo("""{"foo":{"a":1,"b":null}}"""))
      },
      suite("modified")(
        test("modified field") {
          val config = Config.default.withTypes(
            "Query"    -> Config.Type("identity" -> Config.Field.ofType("Identity").resolveWith(Map("a" -> 1))),
            "Identity" -> Config.Type("a" -> Config.Field.ofType("Int").withName("b")),
          )

          for {
            json <- resolve(config, Map.empty)("query {identity {b}}")
          } yield assertTrue(json == """{"identity":{"b":1}}""")
        },
        test("modified argument name") {
          val config = Config.default.withTypes(
            "Query" -> Config.Type(
              "identity" -> Config.Field.int.withArguments("input" -> Config.Arg.int.withName("data"))
                .resolveWithFunction(_.path("args", "data").toDynamic)
            )
          )
          for {
            json <- resolve(config, Map.empty)("""query {identity(data: 1000)}""")
          } yield assertTrue(json == """{"identity":1000}""")
        },
        test("modified input field should not be allowed") {
          val identityType = Config.Type(
            "identity" -> Config.Field.ofType("Identity").withArguments("input" -> Config.Arg.ofType("Identity"))
              .resolveWithFunction(_.path("args", "input").toDynamic)
          )

          val config = Config.default.withTypes(
            "Query"    -> identityType,
            "Identity" -> Config.Type("a" -> Config.Field.ofType("Int").withName("b")),
          )
          for {
            json <- resolve(config, Map.empty)("query {identity(input: {a: 1}){a}}")
          } yield assertTrue(json == """{"identity":{"a":1}}""")
        },
        test("resolve using env variables") {
          val config = Config.default.withTypes(
            "Query" -> Config.Type("identity" -> Config.Field.int.resolveWithFunction(_.path("env", "foo").toDynamic))
          )
          for {
            json <- resolve(config, Map.empty)("""query {identity}""")
          } yield assertTrue(json == """{"identity":"bar"}""")
        },
        test("resolve with headers") {
          val config = Config.default.withTypes(
            "Query" -> Config
              .Type("identity" -> Config.Field.string.resolveWithFunction(_.path("headers", "authorization").toDynamic))
          )
          for {
            json <- resolve(config, Map.empty)("""query {identity}""")
          } yield assertTrue(json == """{"identity":"bar"}""")
        },
      ),
      suite("unsafe")(test("with http") {
        val http   = Operation.Http(Path.unsafe.fromString("/users"))
        val config = Config.default.withBaseURL(URI.create("https://jsonplaceholder.typicode.com").toURL)
          .withTypes("Query" -> Type("foo" -> Config.Field.ofType("Foo").withSteps(http).withHttp(http)))

        val errors = config.toBlueprint.errors

        assertTrue(errors == Chunk("Query.foo can not have unsafe and http operations together"))
      }),
      suite("inline")(
        test("with no base url") {
          val http   = Operation.Http(Path.unsafe.fromString("/users"))
          val config = Config.default.withTypes("Query" -> Type("foo" -> Config.Field.int.withSteps(http)))

          val errors = config.toBlueprint.errors

          assertTrue(errors == Chunk("No base URL defined in the server configuration"))
        },
        test("http directive") {
          val config = Config.default.withBaseURL(URI.create("https://jsonplaceholder.typicode.com").toURL).withTypes(
            "Query" -> Config
              .Type("user" -> Config.Field.ofType("User").withHttp(Operation.Http(Path.unsafe.fromString("/users/1")))),
            "User"  -> Config.Type("id" -> Config.Field.ofType("Int"), "name" -> Config.Field.ofType("String")),
          )

          for {
            json <- resolve(config, Map.empty)("""query {user {id name}}""")
          } yield assertTrue(json == """{"user":{"id":1,"name":"Leanne Graham"}}""")
        },
        test("inline field") {
          val config = Config.default.withTypes(
            "Query" -> Config.Type(
              "foo" -> Config.Field.ofType("Foo").withInline("a", "b")
                .resolveWith(Map("a" -> Map("b" -> Map("c" -> "Hello!"))))
            ),
            "Foo"   -> Config.Type("a" -> Config.Field.ofType("A")),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B")),
            "B"     -> Config.Type("c" -> Config.Field.ofType("String")),
          )

          for {
            json <- resolve(config, Map.empty)("""query {foo {c}}""")
          } yield assertTrue(json == """{"foo":{"c":"Hello!"}}""")
        },
        test("inline with modify field") {
          val config = Config.default.withTypes(
            "Query" -> Config.Type(
              "foo" -> Config.Field.ofType("Foo").withInline("a", "b").withName("bar")
                .resolveWith(Map("a" -> Map("b" -> Map("c" -> "Hello!"))))
            ),
            "Foo"   -> Config.Type("a" -> Config.Field.ofType("A")),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B")),
            "B"     -> Config.Type("c" -> Config.Field.ofType("String")),
          )

          for {
            json <- resolve(config, Map.empty)("""query {bar {c}}""")
          } yield assertTrue(json == """{"bar":{"c":"Hello!"}}""")
        },
        test("inline with list") {
          val config = Config.default.withTypes(
            "Query" -> Config.Type(
              "foo" -> Config.Field.ofType("Foo").withInline("a", "b")
                .resolveWith(Map("a" -> List(Map("b" -> List(Map("c" -> "Hello!"))))))
            ),
            "Foo"   -> Config.Type("a" -> Config.Field.ofType("A").asList),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").asList),
            "B"     -> Config.Type("c" -> Config.Field.ofType("String")),
          )

          for {
            json <- resolve(config, Map.empty)("""query {foo {c}}""")
          } yield assertTrue(json == """{"foo":[[{"c":"Hello!"}]]}""")
        },
        test("inline on index with list") {
          val config = Config.default.withTypes(
            "Query" -> Config.Type(
              "foo" -> Config.Field.ofType("Foo").withInline("a", "0", "b")
                .resolveWith(Map("a" -> List(Map("b" -> List(Map("c" -> "Hello!"))))))
            ),
            "Foo"   -> Config.Type("a" -> Config.Field.ofType("A").asList),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").asList),
            "B"     -> Config.Type("c" -> Config.Field.ofType("String")),
          )

          for {
            json <- resolve(config, Map.empty)("""query {foo {c}}""")
          } yield assertTrue(json == """{"foo":[{"c":"Hello!"}]}""")
        },
      ),
      suite("context value")(
        test("root") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.int.resolveWithFunction(i => i.path("value").debug.toDynamic))
          )
          val expected = "Some(Primitive((),unit))"
          for {
            _   <- resolve(config)("query {a}")
            out <- TestConsole.output
          } yield assertTrue(out == Vector(expected + "\n"))
        },
        test("one level") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").resolveWith(100)),
            "A"     -> Config.Type("b" -> Config.Field.int.resolveWithFunction(_.path("value").debug.toDynamic)),
          )
          val expected = "Some(Primitive(100,int))"
          for {
            _   <- resolve(config)("query {a{b}}")
            out <- TestConsole.output
          } yield assertTrue(out == Vector(expected + "\n"))
        },
        test("one level with list") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").asList.resolveWith(List(100, 200))),
            "A"     -> Config.Type("b" -> Config.Field.int.resolveWithFunction(_.path("value").debug.toDynamic)),
          )
          val expected = Set("Some(Primitive(100,int))\n", "Some(Primitive(200,int))\n")
          for {
            _   <- resolve(config)("query {a{b}}")
            out <- TestConsole.output.map(_.toSet)
          } yield assertTrue(out == expected)
        },
        test("two level") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").resolveWith(100)),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").resolveWith(200)),
            "B"     -> Config.Type("c" -> Config.Field.int.resolveWithFunction(_.path("value").debug.toDynamic)),
          )
          val expected = "Some(Primitive(200,int))"
          for {
            _   <- resolve(config)("query {a{b{c}}}")
            out <- TestConsole.output
          } yield assertTrue(out == Vector(expected + "\n"))
        },
        test("two level with list") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").asList.resolveWith(List(100, 101))),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").asList.resolveWith(List(200, 201))),
            "B"     -> Config.Type("c" -> Config.Field.int.resolveWithFunction(_.path("value").debug.toDynamic)),
          )
          val expected = Set("Some(Primitive(200,int))\n", "Some(Primitive(201,int))\n")
          for {
            _   <- resolve(config)("query {a{b{c}}}")
            out <- TestConsole.output.map(_.toSet)
          } yield assertTrue(out == expected)
        },
      ),
      suite("context parent")(
        test("root") {
          val config   = Config.default.withTypes(
            "Query" -> Config
              .Type("a" -> Config.Field.int.resolveWithFunction(i => i.path("parent", "value").debug.toDynamic))
          )
          val expected = "None"
          for {
            _   <- resolve(config)("query {a}")
            out <- TestConsole.output
          } yield assertTrue(out == Vector(expected + "\n"))
        },
        test("one level") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").resolveWith(100)),
            "A" -> Config.Type("b" -> Config.Field.int.resolveWithFunction(_.path("parent", "value").debug.toDynamic)),
          )
          val expected = "Some(Primitive((),unit))"
          for {
            _   <- resolve(config)("query {a{b}}")
            out <- TestConsole.output
          } yield assertTrue(out == Vector(expected + "\n"))
        },
        test("two level") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").resolveWith(100)),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").resolveWith(200)),
            "B" -> Config.Type("c" -> Config.Field.int.resolveWithFunction(_.path("parent", "value").debug.toDynamic)),
          )
          val expected = "Some(Primitive(100,int))"
          for {
            _   <- resolve(config)("query {a{b{c}}}")
            out <- TestConsole.output
          } yield assertTrue(out == Vector(expected + "\n"))
        },
        test("two level with list") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").asList.resolveWith(List(100, 101))),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").asList.resolveWith(List(200, 201))),
            "B" -> Config.Type("c" -> Config.Field.int.resolveWithFunction(_.path("parent", "value").debug.toDynamic)),
          )
          val expected = Set("Some(Primitive(100,int))\n", "Some(Primitive(101,int))\n")
          for {
            _   <- resolve(config)("query {a{b{c}}}")
            out <- TestConsole.output.map(_.toSet)
          } yield assertTrue(out == expected)
        },
        test("two level with optional list") {
          val config   = Config.default.withTypes(
            "Query" -> Config.Type("a" -> Config.Field.ofType("A").asList.resolveWith(Option(List(100, 101)))),
            "A"     -> Config.Type("b" -> Config.Field.ofType("B").asList.resolveWith(Option(List(200, 201)))),
            "B" -> Config.Type("c" -> Config.Field.int.resolveWithFunction(_.path("parent", "value").debug.toDynamic)),
          )
          val expected = Set("Some(Primitive(100,int))\n", "Some(Primitive(101,int))\n")
          for {
            _   <- resolve(config)("query {a{b{c}}}")
            out <- TestConsole.output.map(_.toSet)
          } yield assertTrue(out == expected)
        },
      ),
    ).provide(
      GraphQLGenerator.default,
      HttpClient.default,
      HttpContext.live(Some(Request.get(ZURL.empty).addHeaders(Headers("authorization", "bar")))),
    ) @@ parallel @@ timeout(10 seconds) @@ before(TestSystem.putEnv("foo", "bar"))

  private def resolve(config: Config, variables: Map[String, InputValue] = Map.empty)(
    query: String
  ): ZIO[HttpContext with GraphQLGenerator, Throwable, String] = {
    for {
      blueprint   <- Transcoder.toBlueprint(config).toTask
      graphQL     <- blueprint.toGraphQL
      interpreter <- graphQL.interpreter
      result      <- interpreter.execute(query, variables = variables)
      _           <- result.errors.headOption match {
        case Some(error) => ZIO.fail(error)
        case None        => ZIO.unit
      }
    } yield result.data.toString
  }
}
