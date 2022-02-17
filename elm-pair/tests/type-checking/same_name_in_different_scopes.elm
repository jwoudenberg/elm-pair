module Main exposing (..)


increment : Int -> Int
increment x =
    x + 1


nextVersion : String -> String
nextVersion x =
    x ++ " 2000"



-- === expected output below ===
-- digraph {
-- "(++)" -> "String -> String -> String" [color = red]
-- "ArgTo((+))" -> "Int" [color = red]
-- "ArgTo((++))" -> "String" [color = red]
-- "ArgTo(ResultOf((++)))" -> "String" [color = red]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(increment)" -> "Int" [color = red]
-- "ArgTo(nextVersion)" -> "String" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "ArgTo(increment)" [dir=none]
-- "Int" -> "Int" [color = red]
-- "Int" -> "ResultOf(increment)" [dir=none]
-- "ResultOf((++))" -> "String -> String" [color = red]
-- "ResultOf(ResultOf((+)))" -> "Int" [color = red]
-- "ResultOf(ResultOf((++)))" -> "String" [color = red]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(increment)" -> "Int" [color = red]
-- "ResultOf(increment)" -> "ResultOf(ResultOf((+)))" [dir=none]
-- "ResultOf(nextVersion)" -> "ResultOf(ResultOf((++)))" [dir=none]
-- "ResultOf(nextVersion)" -> "String" [color = red]
-- "String" -> "ArgTo(ResultOf((++)))" [dir=none]
-- "String" -> "ArgTo(nextVersion)" [dir=none]
-- "String" -> "ResultOf(nextVersion)" [dir=none]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "increment" -> "Int -> Int" [color = red]
-- "increment.x" -> "ArgTo((+))" [dir=none]
-- "increment.x" -> "ArgTo(increment)" [dir=none]
-- "increment.x" -> "Int" [color = red]
-- "nextVersion" -> "String -> String" [color = red]
-- "nextVersion.x" -> "ArgTo((++))" [dir=none]
-- "nextVersion.x" -> "ArgTo(nextVersion)" [dir=none]
-- "nextVersion.x" -> "String" [color = red]
-- }
