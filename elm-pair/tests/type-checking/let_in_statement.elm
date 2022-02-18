module Main exposing (..)


times4 : Int -> Int
times4 number =
    let
        times2 =
            number + number

        times3 : Int
        times3 =
            times2 + number
    in
    times3 + number



-- === expected output below ===
-- digraph {
-- "(+)" -> "Int -> Int -> Int" [color = red]
-- "(+).arg" -> "Int" [color = red]
-- "(+).result" -> "Int -> Int" [color = red]
-- "(+).result.arg" -> "Int" [color = red]
-- "(+).result.result" -> "Int" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "Int" [color = red]
-- "Int" -> "times4.arg" [dir=none]
-- "Int" -> "times4.result" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "String.length.arg" -> "String" [color = red]
-- "String.length.result" -> "Int" [color = red]
-- "times4" -> "Int -> Int" [color = red]
-- "times4.arg" -> "Int" [color = red]
-- "times4.number" -> "(+).arg" [dir=none]
-- "times4.number" -> "(+).result.arg" [dir=none]
-- "times4.number" -> "Int" [color = red]
-- "times4.number" -> "times4.arg" [dir=none]
-- "times4.result" -> "(+).result.result" [dir=none]
-- "times4.result" -> "Int" [color = red]
-- "times4.result.Int" -> "Int" [color = red]
-- "times4.result.Int" -> "times4.result.times3" [dir=none]
-- "times4.result.times2" -> "(+).arg" [dir=none]
-- "times4.result.times2" -> "(+).result.result" [dir=none]
-- "times4.result.times2" -> "Int" [color = red]
-- "times4.result.times3" -> "(+).arg" [dir=none]
-- "times4.result.times3" -> "(+).result.result" [dir=none]
-- "times4.result.times3" -> "Int" [color = red]
-- }
