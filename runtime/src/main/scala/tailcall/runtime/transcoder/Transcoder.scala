package tailcall.runtime.transcoder

import caliban.parsing.adt.Document
import tailcall.runtime.internal.TValid
import tailcall.runtime.model.{Blueprint, Config, Endpoint}
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
    with GraphQLSchema2JsonLines
    with JsonValue2TSchema
    with Orc2Blueprint
    with ToDynamicValue
    with ToInputValue
    with ToJsonAST
    with ToResponseValue
    with ToValue

object Transcoder extends Transcoder {
  def toBlueprint(endpoint: Endpoint, encodeDirectives: Boolean, nameGen: NameGenerator): TValid[String, Blueprint] =
    toConfig(endpoint, nameGen).flatMap(toBlueprint(_, encodeDirectives))

  def toGraphQLSchema(blueprint: Blueprint): TValid[Nothing, String] = toDocument(blueprint).flatMap(toGraphQLSchema(_))

  def toGraphQLSchema(
    endpoint: Endpoint,
    encodeDirectives: Boolean,
    nameGenerator: NameGenerator,
  ): TValid[String, String] =
    toConfig(endpoint, nameGenerator).flatMap(config => toGraphQLSchema(config.compress, encodeDirectives))

  def toGraphQLSchema(config: Config, encodeDirectives: Boolean): TValid[Nothing, String] =
    toDocument(config, encodeDirectives).flatMap(toGraphQLSchema(_))

  def toDocument(config: Config, encodeDirectives: Boolean): TValid[Nothing, Document] =
    for {
      blueprint <- toBlueprint(config, encodeDirectives = encodeDirectives)
      document  <- toDocument(blueprint)
    } yield document
}
