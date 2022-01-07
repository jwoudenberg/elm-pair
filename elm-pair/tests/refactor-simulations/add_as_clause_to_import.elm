module Main exposing (..)

import Json.Decode exposing (map2)


sumDecoder : Json.Decode.Decoder Int
sumDecoder =
    map2 (+)
        (Json.Decode.field "x" Json.Decode.int)
        (Json.Decode.field "y" Json.Decode.int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 exposing
-- DELETE exposing
-- INSERT as Dec exposing
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode as Dec exposing (map2)
--
--
-- sumDecoder : Dec.Decoder Int
-- sumDecoder =
--     map2 (+)
--         (Dec.field "x" Dec.int)
--         (Dec.field "y" Dec.int)
