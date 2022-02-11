module Main exposing (..)


foo : String -> Int
foo str =
    String.length str



-- === expected output below ===
-- ArgTo(Name(foo)) `SameAs` Name(String)
-- ArgTo(Name(foo)) `ArgTo` Name(foo)
-- ResultOf(Name(foo)) `SameAs` Name(Int)
-- ResultOf(Name(foo)) `ResultOf` Name(foo)
-- Name(str) `ArgTo` Name(foo)
-- Name(str) `ArgTo` Name(String.length)
-- ResultOf(Name(foo)) `SameAs` ResultOf(Name(String.length))
-- ResultOf(Name(foo)) `ResultOf` Name(String.length)
