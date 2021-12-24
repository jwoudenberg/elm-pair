module Main exposing (..)

import Json.Decode as Dec exposing (map2)


sumDecoder : Dec.Decoder Int
sumDecoder =
    map2 (+)
        (Dec.field "x" Dec.int)
        (Dec.field "y" Dec.int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 as Dec
-- DELETE as Dec
-- INSERT as Json
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode as Json exposing (map2)
--
--
-- sumDecoder : Json.Decoder Int
-- sumDecoder =
--     map2 (+)
--         (Json.field "x" Json.int)
--         (Json.field "y" Json.int)
