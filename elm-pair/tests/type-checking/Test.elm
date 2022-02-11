module Main exposing (..)


foo : String -> String -> Int
foo strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- ArgTo((++)) `SameAs` strA
-- ArgTo(ResultOf((++))) `SameAs` strB
-- ArgTo(ResultOf(foo)) `SameAs` String
-- ArgTo(foo) `SameAs` String
-- ResultOf(ResultOf((++))) `SameAs` ArgTo(String.length)
-- ResultOf(ResultOf(foo)) `SameAs` Int
-- ResultOf(ResultOf(foo)) `SameAs` ResultOf(String.length)
-- strA `SameAs` ArgTo(foo)
-- strB `SameAs` ArgTo(ResultOf(foo))
