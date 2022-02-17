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
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(max10)" -> "Int" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "Bool" -> "IfCond(ResultOf(max10))" [dir=none]
-- "IfCond(ResultOf(max10))" -> "Bool" [color = red]
-- "IfFalse(ResultOf(max10))" -> "Int" [color = red]
-- "IfTrue(ResultOf(max10))" -> "Int" [color = red]
-- "Int" -> "ArgTo(max10)" [dir=none]
-- "Int" -> "Int" [color = red]
-- "Int" -> "ResultOf(max10)" [dir=none]
-- "ResultOf(ResultOf((>)))" -> "Bool" [color = red]
-- "ResultOf(ResultOf((>)))" -> "IfCond(ResultOf(max10))" [dir=none]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(max10)" -> "IfFalse(ResultOf(max10))" [dir=none]
-- "ResultOf(max10)" -> "IfTrue(ResultOf(max10))" [dir=none]
-- "ResultOf(max10)" -> "Int" [color = red]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "max10" -> "Int -> Int" [color = red]
-- "max10.number" -> "ArgTo((>))" [dir=none]
-- "max10.number" -> "ArgTo(max10)" [dir=none]
-- "max10.number" -> "IfFalse(ResultOf(max10))" [dir=none]
-- "max10.number" -> "Int" [color = red]
-- }
