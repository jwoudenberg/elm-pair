module ModuleUsingConstructorQualified exposing (..)

import ModuleExposingConstructor


snore : ModuleExposingConstructor.Snore
snore =
    ModuleExposingConstructor.Zzz



-- === expected output below ===
-- module ModuleUsingConstructorQualified exposing (..)
--
-- import ModuleExposingConstructor
--
--
-- snore : ModuleExposingConstructor.Snore
-- snore =
--     ModuleExposingConstructor.SleepySounds
