package tailcall.server

import tailcall.registry.SchemaRegistry
import tailcall.runtime.ast.Digest
import tailcall.runtime.ast.Digest.Algorithm
import tailcall.runtime.service.DataLoader
import tailcall.server.internal.GraphQLUtils
import zio._
import zio.http._
import zio.http.model.{HttpError, Method}
import zio.json.EncoderOps

object GenericServer {
  def graphQL =
    Http.collectZIO[Request] { case req @ Method.POST -> !! / "graphql" / alg / id =>
      for {
        alg         <- Algorithm.fromString(alg) match {
          case Some(value) => ZIO.succeed(value)
          case None        => ZIO.fail(HttpError.BadRequest("Invalid algorithm"))
        }
        digest = Digest.fromHex(alg, id)
        schema      <- SchemaRegistry.get(digest)
        result      <- schema match {
          case Some(value) => value.toGraphQL
          case None        => ZIO.fail(HttpError.NotFound(s"Schema ${id} not found"))
        }
        query       <- GraphQLUtils.decodeQuery(req.body)
        interpreter <- result.interpreter
        res         <- interpreter.execute(query).provideLayer(DataLoader.http)
      } yield Response.json(res.toJson)
    }
}
