module Main exposing (..)

import Json.Decode


sumDecoder : Json.Decode.Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (Json.Decode.field "x" Json.Decode.int)
        (Json.Decode.field "y" Json.Decode.int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 Json.
-- DELETE Json.Decode.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (field)
--
--
-- sumDecoder : Json.Decode.Decoder Int
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (field "x" Json.Decode.int)
--         (field "y" Json.Decode.int)
