module Main exposing (..)


foo : String -> Int
foo str =
    String.length str



-- === expected output below ===
-- ArgTo("foo") `SameAs` Name("String")
-- ArgTo("foo") `ArgTo` Name("foo")
-- ResultOf("foo") `SameAs` Name("Int")
-- ResultOf("foo") `ResultOf` Name("foo")
-- Name("str") `ArgTo` Name("foo")
-- Name("str") `ArgTo` Name("String.length")
-- ResultOf("foo") `SameAs` ResultOf("String.length")
-- ResultOf("foo") `ResultOf` Name("String.length")
