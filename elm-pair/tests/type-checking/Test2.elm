module Main exposing (..)


foo : String -> String -> Int
foo strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- foo : String -> String -> Int
-- String : String
-- Int : Int
-- strA : String
-- strB : String
-- String.length : String -> Int
-- (++) : String -> String -> String
