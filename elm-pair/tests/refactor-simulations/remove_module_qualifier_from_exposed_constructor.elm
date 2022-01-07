module Main exposing (..)

import Parser.Advanced exposing (Nestable(..))


toggleNestable : Nestable -> Nestable
toggleNestable nestable =
    case nestable of
        Parser.Advanced.Nestable ->
            Parser.Advanced.NotNestable

        Parser.Advanced.NotNestable ->
            Parser.Advanced.Nestable



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 Parser
-- DELETE Parser.Advanced.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Parser.Advanced exposing (Nestable(..))
--
--
-- toggleNestable : Nestable -> Nestable
-- toggleNestable nestable =
--     case nestable of
--         Nestable ->
--             NotNestable
--
--         NotNestable ->
--             Nestable
