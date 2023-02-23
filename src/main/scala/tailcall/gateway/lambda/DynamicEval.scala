package tailcall.gateway.lambda

import tailcall.gateway.service.EvaluationContext.Binding
import zio.schema.{DeriveSchema, DynamicValue, Schema}

sealed trait DynamicEval

object DynamicEval {
  // scalafmt: { maxColumn = 240 }
  case object Identity                                                                  extends DynamicEval
  final case class Lookup(binding: Binding)                                             extends DynamicEval
  final case class Immediate(value: DynamicEval)                                        extends DynamicEval
  final case class Defer(value: DynamicEval)                                            extends DynamicEval
  final case class FunctionDef(binding: Binding, body: DynamicEval, input: DynamicEval) extends DynamicEval
  final case class Literal(value: DynamicValue, ctor: Constructor[Any])                 extends DynamicEval
  final case class Pipe(left: DynamicEval, right: DynamicEval)                          extends DynamicEval
  final case class EqualTo(left: DynamicEval, right: DynamicEval, tag: Equatable[Any])  extends DynamicEval
  final case class Math(operation: Math.Operation, tag: Numeric[Any])                   extends DynamicEval
  object Math {
    sealed trait Operation

    final case class Binary(operation: Binary.Operation, left: DynamicEval, right: DynamicEval) extends Operation
    object Binary {
      sealed trait Operation
      case object Add              extends Operation
      case object Multiply         extends Operation
      case object Divide           extends Operation
      case object Modulo           extends Operation
      case object GreaterThan      extends Operation
      case object GreaterThanEqual extends Operation
    }

    final case class Unary(operation: Unary.Operation, value: DynamicEval) extends Operation
    object Unary {
      sealed trait Operation
      case object Negate extends Operation
    }
  }

  final case class Logical(operation: Logical.Operation) extends DynamicEval
  object Logical {
    sealed trait Operation
    final case class Binary(operation: Binary.Operation, left: DynamicEval, right: DynamicEval) extends Operation
    object Binary {
      sealed trait Operation
      case object And extends Operation
      case object Or  extends Operation
    }

    final case class Unary(value: DynamicEval, operation: Unary.Operation) extends Operation
    object Unary {
      sealed trait Operation
      case object Not                                                     extends Operation
      final case class Diverge(isTrue: DynamicEval, isFalse: DynamicEval) extends Operation
    }
  }

  implicit val schema: Schema[DynamicEval] = DeriveSchema.gen[DynamicEval]
}
