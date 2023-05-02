package tailcall.runtime

import tailcall.runtime.service.DataLoader
import zio._
import zio.test.TestAspect.{nonFlaky, timeout}
import zio.test._

import java.io.IOException

object DataLoaderSpec extends ZIOSpecDefault {
  private val failsFirstDL = for {
    ref <- Ref.make(true)
    dl  <- DataLoader.one[Int] { _ =>
      for {
        fail <- ref.get
        _    <- ref.set(false)
        _    <- ZIO.fail("Failure").when(fail)
      } yield "Ok"
    }
  } yield dl

  def spec =
    suite("DataLoaderSpec")(
      test("fail first") {
        for {
          dl <- failsFirstDL
          f1 <- dl.collect(1)
          _  <- dl.dispatch
          r1 <- f1.either
          f2 <- dl.collect(1)
          _  <- dl.dispatch
          r2 <- f2.either
        } yield assertTrue(r1 == Left("Failure") && r2 == Right("Ok"))
      } @@ nonFlaky,
      test("should cache") {
        val value: UIO[DataLoader[Any, IOException, Int, Int]] = DataLoader
          .one[Int](value => zio.Console.printLine("Load") *> ZIO.succeed(value + 1))
        for {
          dl  <- value
          f1  <- dl.collect(1)
          _   <- dl.dispatch
          f2  <- dl.collect(1)
          _   <- dl.dispatch
          r1  <- f1
          r2  <- f2
          out <- TestConsole.output
        } yield assertTrue(r1 == 2 && r2 == 2, out == Vector("Load\n"))

      },
      test("batch") {
        val value: UIO[DataLoader[Any, Nothing, Int, Int]] = DataLoader
          .many[Int](chunk => ZIO.succeed(chunk.map(_ + 1)))

        for {
          dl <- value
          f  <- dl.collect(1, 2, 3, 4)
          _  <- dl.dispatch
          r  <- ZIO.foreach(f)(identity)
        } yield assertTrue(r == Chunk(2, 3, 4, 5))
      },
    ) @@ timeout(5 seconds)

}
