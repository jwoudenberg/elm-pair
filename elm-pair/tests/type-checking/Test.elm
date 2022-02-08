module Main exposing (..)


foo : String -> Int
foo str =
    String.length str



-- === expected output below ===
-- Relations
-- /ModuleRoot/Name("foo")/LambdaArg `SameAs` /ModuleRoot/Name("String")
-- /ModuleRoot/Name("foo")/LambdaArg `ArgTo` /ModuleRoot/Name("foo")
-- /ModuleRoot/Name("foo")/LambdaRes `SameAs` /ModuleRoot/Name("Int")
-- /ModuleRoot/Name("foo")/LambdaRes `ResultOf` /ModuleRoot/Name("foo")
-- /ModuleRoot/Name("foo")/Name("str") `ArgTo` /ModuleRoot/Name("foo")
-- /ModuleRoot/Name("str") `ArgTo` /ModuleRoot/Name("String.length")
-- /ModuleRoot/Name("foo")/LambdaRes `SameAs` /ModuleRoot/Name("String.length")/LambdaRes
-- /ModuleRoot/Name("foo")/LambdaRes `ResultOf` /ModuleRoot/Name("String.length")
