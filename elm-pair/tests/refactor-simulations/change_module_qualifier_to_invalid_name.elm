module Main exposing (..)

import Json.Decode as Dec exposing (int)


sumDecoder : Dec.Decoder Int
sumDecoder =
    Dec.map2 (+)
        (Dec.field "x" int)
        (Dec.field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 8 Dec
-- DELETE Dec
-- INSERT Invalid.Alias
-- END SIMULATION
-- === expected output below ===
-- Refactor produced invalid code:
-- module Main exposing (..)
--
-- import Json.Decode as Invalid.Alias exposing (int)
--
--
-- sumDecoder : Invalid.Alias.Decoder Int
-- sumDecoder =
--     Invalid.Alias.map2 (+)
--         (Invalid.Alias.field "x" int)
--         (Invalid.Alias.field "y" int)
