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
-- "ArgTo((+))" -> "Int" [color = red]
-- "ArgTo(ResultOf((+)))" -> "Int" [color = red]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(times4)" -> "Int" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "ArgTo(times4)" [dir=none]
-- "Int" -> "Int" [color = red]
-- "Int" -> "ResultOf(times4)" [dir=none]
-- "ResultOf((+))" -> "Int -> Int" [color = red]
-- "ResultOf(ResultOf((+)))" -> "Int" [color = red]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(times4)" -> "Int" [color = red]
-- "ResultOf(times4)" -> "ResultOf(ResultOf((+)))" [dir=none]
-- "ResultOf(times4).Int" -> "Int" [color = red]
-- "ResultOf(times4).Int" -> "ResultOf(times4).times3" [dir=none]
-- "ResultOf(times4).times2" -> "ArgTo((+))" [dir=none]
-- "ResultOf(times4).times2" -> "Int" [color = red]
-- "ResultOf(times4).times2" -> "ResultOf(ResultOf((+)))" [dir=none]
-- "ResultOf(times4).times3" -> "ArgTo((+))" [dir=none]
-- "ResultOf(times4).times3" -> "Int" [color = red]
-- "ResultOf(times4).times3" -> "ResultOf(ResultOf((+)))" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "times4" -> "Int -> Int" [color = red]
-- "times4.number" -> "ArgTo((+))" [dir=none]
-- "times4.number" -> "ArgTo(ResultOf((+)))" [dir=none]
-- "times4.number" -> "ArgTo(times4)" [dir=none]
-- "times4.number" -> "Int" [color = red]
-- }
