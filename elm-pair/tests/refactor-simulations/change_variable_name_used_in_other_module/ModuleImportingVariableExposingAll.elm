module ModuleImportingVariableExposingAll exposing (..)

import ModuleExposingVariable exposing (..)


greetWorld : String
greetWorld =
    greeting ++ ", World!"



-- === expected output below ===
-- module ModuleImportingVariableExposingAll exposing (..)
--
-- import ModuleExposingVariable exposing (..)
--
--
-- greetWorld : String
-- greetWorld =
--     greetz ++ ", World!"
