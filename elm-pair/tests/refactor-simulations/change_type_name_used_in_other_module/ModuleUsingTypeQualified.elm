module ModuleUsingTypeQualified exposing (..)

import ModuleExposingType


snore : ModuleExposingType.Snore
snore =
    ModuleExposingType.Zzz



-- === expected output below ===
-- module ModuleUsingTypeQualified exposing (..)
--
-- import ModuleExposingType
--
--
-- snore : ModuleExposingType.SleepySounds
-- snore =
--     ModuleExposingType.Zzz
