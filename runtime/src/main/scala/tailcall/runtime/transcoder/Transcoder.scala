package tailcall.runtime.transcoder

import caliban.parsing.adt.Document
import tailcall.runtime.ast.{Blueprint, Endpoint}
import tailcall.runtime.dsl.{Config, Postman}
import tailcall.runtime.internal.TValid
import tailcall.runtime.transcoder.Endpoint2Config.NameGenerator
import tailcall.runtime.transcoder.value._

/**
 * A transcoder is a function that takes an A and returns a
 * B, or an error. It can be composed using the >>> operator
 * with other transcoders to create a pipeline. A transcoder
 * between A ~> C can be derived provided there exists a B
 * such that a transcoder from A ~> B exists and a
 * transcoder from B ~> C already exists.
 */
sealed trait Transcoder
    extends Blueprint2Document
    with Config2Blueprint
    with Document2Blueprint
    with Document2Config
    with Document2GraphQLSchema
    with Endpoint2Config
    with JsonValue2TSchema
    with Orc2Blueprint
    with Postman2Endpoints
    with ToDynamicValue
    with ToInputValue
    with ToJsonAST
    with ToResponseValue
    with ToValue

object Transcoder extends Transcoder {
  def toBlueprint(endpoint: Endpoint, nameGen: NameGenerator): TValid[String, Blueprint] =
    toConfig(endpoint, nameGen).flatMap(toBlueprint(_))

  def toConfig(postman: Postman): TValid[String, Config] =
    for {
      endpoints <- toEndpoints(postman)
      config    <- TValid.foreach(endpoints)(endpoint => toConfig(endpoint)).map(_.reduce(_ mergeRight _))
    } yield config

  def toGraphQLSchema(endpoint: Endpoint, nameGenerator: NameGenerator): TValid[String, String] =
    toConfig(endpoint, nameGenerator).flatMap(toGraphQLSchema(_))

  def toGraphQLSchema(config: Config): TValid[Nothing, String] = toDocument(config).flatMap(toGraphQLSchema(_))

  def toDocument(config: Config): TValid[Nothing, Document] =
    for {
      blueprint <- toBlueprint(config, encodeSteps = true)
      document  <- toDocument(blueprint)
    } yield document

  def toGraphQLSchema(blueprint: Blueprint): TValid[Nothing, String] = toDocument(blueprint).flatMap(toGraphQLSchema(_))
}
