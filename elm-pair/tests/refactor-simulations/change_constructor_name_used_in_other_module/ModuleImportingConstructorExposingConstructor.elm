module ModuleImportingConstructorExposingConstructor exposing (..)

import ModuleExposingConstructor exposing (Snore(..))


snore : Snore
snore =
    Zzz



-- === expected output below ===
-- module ModuleImportingConstructorExposingConstructor exposing (..)
--
-- import ModuleExposingConstructor exposing (Snore(..))
--
--
-- snore : Snore
-- snore =
--     SleepySounds
