module Main exposing (..)

import Json.Decode exposing (Decoder, field)


sumDecoder : Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (field "x" Json.Decode.int)
        (field "y" Json.Decode.int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 9 Json.
-- DELETE Json.Decode.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, field, int)
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (field "x" int)
--         (field "y" int)
