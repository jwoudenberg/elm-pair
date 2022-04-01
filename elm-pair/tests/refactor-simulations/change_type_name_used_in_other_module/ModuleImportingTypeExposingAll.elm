module ModuleImportingTypeExposingAll exposing (..)

import ModuleExposingType exposing (..)


snore : Snore
snore =
    Zzz



-- === expected output below ===
-- module ModuleImportingTypeExposingAll exposing (..)
--
-- import ModuleExposingType exposing (..)
--
--
-- snore : SleepySounds
-- snore =
--     Zzz
