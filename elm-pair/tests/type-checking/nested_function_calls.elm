module Main exposing (..)


combinedLength : String -> String -> Int
combinedLength strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- digraph {
-- "(++)" -> "String -> String -> String" [color = red]
-- "ArgTo((++))" -> "String" [color = red]
-- "ArgTo((++))" -> "strA" [dir=none]
-- "ArgTo(ResultOf((++)))" -> "String" [color = red]
-- "ArgTo(ResultOf((++)))" -> "strB" [dir=none]
-- "ArgTo(ResultOf(combinedLength))" -> "String" [color = red]
-- "ArgTo(ResultOf(combinedLength))" -> "String" [dir=none]
-- "ArgTo(String.length)" -> "String" [color = red]
-- "ArgTo(combinedLength)" -> "String" [color = red]
-- "ArgTo(combinedLength)" -> "String" [dir=none]
-- "Bool" -> "Bool" [color = red]
-- "Int" -> "Int" [color = red]
-- "ResultOf((++))" -> "String -> String" [color = red]
-- "ResultOf(ResultOf((++)))" -> "ArgTo(String.length)" [dir=none]
-- "ResultOf(ResultOf((++)))" -> "String" [color = red]
-- "ResultOf(ResultOf(combinedLength))" -> "Int" [color = red]
-- "ResultOf(ResultOf(combinedLength))" -> "Int" [dir=none]
-- "ResultOf(ResultOf(combinedLength))" -> "ResultOf(String.length)" [dir=none]
-- "ResultOf(String.length)" -> "Int" [color = red]
-- "ResultOf(combinedLength)" -> "String -> Int" [color = red]
-- "String" -> "String" [color = red]
-- "String.length" -> "String -> Int" [color = red]
-- "combinedLength" -> "String -> String -> Int" [color = red]
-- "strA" -> "ArgTo(combinedLength)" [dir=none]
-- "strA" -> "String" [color = red]
-- "strB" -> "ArgTo(ResultOf(combinedLength))" [dir=none]
-- "strB" -> "String" [color = red]
-- }
