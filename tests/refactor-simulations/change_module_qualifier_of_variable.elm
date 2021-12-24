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
-- INSERT Json
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode as Json exposing (int)
--
--
-- sumDecoder : Json.Decoder Int
-- sumDecoder =
--     Json.map2 (+)
--         (Json.field "x" int)
--         (Json.field "y" int)
