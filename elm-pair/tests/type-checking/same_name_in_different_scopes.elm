module Main exposing (..)


increment : Int -> Int
increment x =
    x + 1


nextVersion : String -> String
nextVersion x =
    x ++ " 2000"



-- === expected output below ===
-- digraph {
-- "(+).arg" -> "Int" [color = red]
-- "(+).result.result" -> "Int" [color = red]
-- "(++)" -> "String -> String -> String" [color = red]
-- "(++).arg" -> "String" [color = red]
-- "(++).result" -> "String -> String" [color = red]
-- "(++).result.arg" -> "String" [color = red]
-- "(++).result.result" -> "String" [color = red]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "Int" [color = red]
-- "Int" -> "increment.arg" [dir=none]
-- "Int" -> "increment.result" [dir=none]
-- "String" -> "(++).result.arg" [dir=none]
-- "String" -> "String" [color = red]
-- "String" -> "nextVersion.arg" [dir=none]
-- "String" -> "nextVersion.result" [dir=none]
-- "String.length" -> "String -> Int" [color = red]
-- "String.length.arg" -> "String" [color = red]
-- "String.length.result" -> "Int" [color = red]
-- "increment" -> "Int -> Int" [color = red]
-- "increment.arg" -> "Int" [color = red]
-- "increment.result" -> "(+).result.result" [dir=none]
-- "increment.result" -> "Int" [color = red]
-- "increment.x" -> "(+).arg" [dir=none]
-- "increment.x" -> "Int" [color = red]
-- "increment.x" -> "increment.arg" [dir=none]
-- "nextVersion" -> "String -> String" [color = red]
-- "nextVersion.arg" -> "String" [color = red]
-- "nextVersion.result" -> "(++).result.result" [dir=none]
-- "nextVersion.result" -> "String" [color = red]
-- "nextVersion.x" -> "(++).arg" [dir=none]
-- "nextVersion.x" -> "String" [color = red]
-- "nextVersion.x" -> "nextVersion.arg" [dir=none]
-- }
