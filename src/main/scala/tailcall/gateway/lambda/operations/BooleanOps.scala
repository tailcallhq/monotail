package tailcall.gateway.lambda.operations

import tailcall.gateway.lambda.DynamicEval.Logical
import tailcall.gateway.lambda.Remote

trait BooleanOps {
  implicit final class RemoteBooleanOps(val self: Remote[Boolean]) {
    def &&(other: Remote[Boolean]): Remote[Boolean] =
      Remote
        .unsafe
        .attempt(ctx =>
          Logical(
            Logical
              .Binary(self.compile(ctx), other.compile(ctx), Logical.Binary.And)
          )
        )

    def ||(other: Remote[Boolean]): Remote[Boolean] =
      Remote
        .unsafe
        .attempt(ctx =>
          Logical(
            Logical
              .Binary(self.compile(ctx), other.compile(ctx), Logical.Binary.Or)
          )
        )

    def unary_! : Remote[Boolean] =
      Remote
        .unsafe
        .attempt(ctx =>
          Logical(Logical.Unary(self.compile(ctx), Logical.Unary.Not))
        )

    def diverge[A](isTrue: Remote[A], isFalse: Remote[A]): Remote[A] =
      Remote
        .unsafe
        .attempt(ctx =>
          Logical(Logical.Unary(
            self.compile(ctx),
            Logical.Unary.Diverge(isTrue.compile(ctx), isFalse.compile(ctx))
          ))
        )
  }
}
