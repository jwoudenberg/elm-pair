module Main exposing (..)

import Parser.Advanced as Parse exposing (Nestable)


toggleNestable : Nestable -> Nestable
toggleNestable nestable =
    case nestable of
        Parse.Nestable ->
            Parse.NotNestable

        Parse.NotNestable ->
            Parse.Nestable



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 Parse
-- DELETE Parse
-- INSERT Grok
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Parser.Advanced as Grok exposing (Nestable)
--
--
-- toggleNestable : Nestable -> Nestable
-- toggleNestable nestable =
--     case nestable of
--         Grok.Nestable ->
--             Grok.NotNestable
--
--         Grok.NotNestable ->
--             Grok.Nestable
