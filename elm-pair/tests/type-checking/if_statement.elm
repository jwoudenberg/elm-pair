module Main exposing (..)


max10 : Int -> Int
max10 number =
    if number > 10 then
        10

    else
        number



-- === expected output below ===
-- digraph {
-- "ArgTo((>))" -> "Int" [color = red]
-- "ArgTo((>))" -> "number" [dir=none]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(max10)" -> "Int" [color = red]
-- "ArgTo(max10)" -> "Int" [dir=none]
-- "Bool" -> "Bool" [color = red]
-- "IfCond(ResultOf(max10))" -> "Bool" [color = red]
-- "IfCond(ResultOf(max10))" -> "Bool" [dir=none]
-- "IfFalse(ResultOf(max10))" -> "Int" [color = red]
-- "IfFalse(ResultOf(max10))" -> "ResultOf(max10)" [dir=none]
-- "IfFalse(ResultOf(max10))" -> "number" [dir=none]
-- "IfTrue(ResultOf(max10))" -> "Int" [color = red]
-- "IfTrue(ResultOf(max10))" -> "ResultOf(max10)" [dir=none]
-- "Int" -> "Int" [color = red]
-- "ResultOf(ResultOf((>)))" -> "Bool" [color = red]
-- "ResultOf(ResultOf((>)))" -> "IfCond(ResultOf(max10))" [dir=none]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(max10)" -> "Int" [color = red]
-- "ResultOf(max10)" -> "Int" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "max10" -> "Int -> Int" [color = red]
-- "number" -> "ArgTo(max10)" [dir=none]
-- "number" -> "Int" [color = red]
-- }
