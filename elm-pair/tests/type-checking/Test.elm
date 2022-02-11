module Main exposing (..)


foo : String -> String -> Int
foo strA strB =
    String.length (strA ++ strB)



-- === expected output below ===
-- ArgTo(Name(foo)) `SameAs` Name(String)
-- ArgTo(Name(foo)) `ArgTo` Name(foo)
-- ResultOf(Name(foo)) `ResultOf` Name(foo)
-- ArgTo(ResultOf(Name(foo))) `SameAs` Name(String)
-- ArgTo(ResultOf(Name(foo))) `ArgTo` ResultOf(Name(foo))
-- ResultOf(ResultOf(Name(foo))) `ResultOf` ResultOf(Name(foo))
-- ResultOf(ResultOf(Name(foo))) `SameAs` Name(Int)
-- Name(strA) `ArgTo` Name(foo)
-- Name((strA ++ strB)) `ArgTo` Name(String.length)
-- ResultOf(Name(foo)) `SameAs` ResultOf(Name(String.length))
-- ResultOf(Name(foo)) `ResultOf` Name(String.length)
