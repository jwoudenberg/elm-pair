module ModuleImportingTwoTypesWithSameNameExposingOne exposing (..)

import BookOne
import BookTwo exposing (Model)


type Book
    = One BookOne.Model
    | Two Model



-- === expected output below ===
-- module ModuleImportingTwoTypesWithSameNameExposingOne exposing (..)
--
-- import BookOne exposing (Model)
-- import BookTwo
--
--
-- type Book
--     = One Model
--     | Two BookTwo.Model
