module ModuleImportingTypeExposingType exposing (..)

import ModuleExposingType exposing (Snore(..))


snore : Snore
snore =
    Zzz



-- === expected output below ===
-- module ModuleImportingTypeExposingType exposing (..)
--
-- import ModuleExposingType exposing (SleepySounds(..))
--
--
-- snore : SleepySounds
-- snore =
--     Zzz
