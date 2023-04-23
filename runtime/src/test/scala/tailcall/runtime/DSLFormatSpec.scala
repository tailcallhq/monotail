package tailcall.runtime

import tailcall.runtime.model.Config.{Field, Type}
import tailcall.runtime.model.{Config, Path, Step}
import tailcall.runtime.service.DSLFormat
import zio.ZIO
import zio.test._

object DSLFormatSpec extends ZIOSpecDefault {
  private def assertGraphQL(str: String, config: Config): ZIO[Any, String, TestResult] =
    for {
      graphQL  <- DSLFormat.GRAPHQL.decode(str)
      actual   <- graphQL.asJSONConfig
      expected <- config.asJSONConfig
    } yield assertTrue(actual == expected)

  def spec =
    suite("GraphQL")(test("http directive") {
      val doc = """
                  |type User {
                  | id: Int
                  | name: String
                  |}
                  |
                  |type Query {
                  |  foo: [User] @http(path: "/users")
                  |}
                  |""".stripMargin

      val expected = Config.empty.withTypes(
        "User"  -> Type("id" -> Field.ofType("Int"), "name" -> Field.ofType("String")),
        "Query" -> Type("foo" -> Field.ofType("User").asList.withSteps(Step.Http(Path.unsafe.fromString("/users")))),
      )

      assertGraphQL(doc, expected)
    })

}
