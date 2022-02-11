module Main exposing (..)


foo : String -> String -> Int
foo strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- ArgTo((++)) `ArgTo` (++)
-- ArgTo((++)) `SameAs` strA
-- ArgTo(ResultOf((++))) `ArgTo` ResultOf((++))
-- ArgTo(ResultOf((++))) `SameAs` strB
-- ArgTo(ResultOf(foo)) `ArgTo` ResultOf(foo)
-- ArgTo(ResultOf(foo)) `SameAs` String
-- ArgTo(String.length) `ArgTo` String.length
-- ArgTo(foo) `ArgTo` foo
-- ArgTo(foo) `SameAs` String
-- ResultOf((++)) `ResultOf` (++)
-- ResultOf(ResultOf((++))) `ResultOf` ResultOf((++))
-- ResultOf(ResultOf((++))) `SameAs` ArgTo(String.length)
-- ResultOf(ResultOf(foo)) `ResultOf` ResultOf(foo)
-- ResultOf(ResultOf(foo)) `SameAs` Int
-- ResultOf(ResultOf(foo)) `SameAs` ResultOf(String.length)
-- ResultOf(foo) `ResultOf` foo
-- String.length `SameAs` String.length
-- strA `ArgTo` foo
-- strB `ArgTo` ResultOf(foo)
