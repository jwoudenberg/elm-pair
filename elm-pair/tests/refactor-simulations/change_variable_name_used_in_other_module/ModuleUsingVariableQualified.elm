module ModuleUsingVariableQualified exposing (..)

import ModuleExposingVariable


greetWorld : String
greetWorld =
    ModuleExposingVariable.greeting ++ ", World!"



-- === expected output below ===
-- module ModuleUsingVariableQualified exposing (..)
--
-- import ModuleExposingVariable
--
--
-- greetWorld : String
-- greetWorld =
--     ModuleExposingVariable.greetz ++ ", World!"
