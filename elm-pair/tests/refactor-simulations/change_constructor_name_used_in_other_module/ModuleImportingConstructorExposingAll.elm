module ModuleImportingConstructorExposingAll exposing (..)

import ModuleExposingConstructor exposing (..)


snore : Snore
snore =
    Zzz



-- === expected output below ===
-- module ModuleImportingConstructorExposingAll exposing (..)
--
-- import ModuleExposingConstructor exposing (..)
--
--
-- snore : Snore
-- snore =
--     SleepySounds
