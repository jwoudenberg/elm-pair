module Main exposing (..)

import Parser.Advanced exposing (Nestable(..), Parser)


toggleNestable : Nestable -> Nestable
toggleNestable nestable =
    case nestable of
        Nestable ->
            NotNestable

        NotNestable ->
            Nestable



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 Nestable
-- DELETE Nestable(..),
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Parser.Advanced exposing ( Parser)
--
--
-- toggleNestable : Parser.Advanced.Nestable -> Parser.Advanced.Nestable
-- toggleNestable nestable =
--     case nestable of
--         Parser.Advanced.Nestable ->
--             Parser.Advanced.NotNestable
--
--         Parser.Advanced.NotNestable ->
--             Parser.Advanced.Nestable
