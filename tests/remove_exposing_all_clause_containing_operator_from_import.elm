module Main exposing (..)

import Parser exposing (..)


add : Parser Int
add =
    Parser.succeed (+)
        |= int
        |= int



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 exposing
-- DELETE exposing (..)
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Parser
--
--
-- add : Parser.Parser Int
-- add =
--     Parser.succeed (+)
--         |= Parser.int
--         |= Parser.int
