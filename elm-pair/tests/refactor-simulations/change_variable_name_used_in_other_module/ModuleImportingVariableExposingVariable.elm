module ModuleImportingVariableExposingVariable exposing (..)

import ModuleExposingVariable exposing (greeting)


greetWorld : String
greetWorld =
    greeting ++ ", World!"



-- === expected output below ===
-- module ModuleImportingVariableExposingVariable exposing (..)
--
-- import ModuleExposingVariable exposing (greetz)
--
--
-- greetWorld : String
-- greetWorld =
--     greetz ++ ", World!"
