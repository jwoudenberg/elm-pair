module ModuleImportingTwoTypesWithSameName exposing (..)

import PageOne
import PageTwo


type Page
    = One PageOne.Model
    | Two PageTwo.Model



-- === expected output below ===
-- module ModuleImportingTwoTypesWithSameName exposing (..)
--
-- import PageOne
-- import PageTwo exposing (Model)
--
--
-- type Page
--     = One PageOne.Model
--     | Two Model
