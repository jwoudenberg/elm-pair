module Math exposing (..)

import Json.Decode exposing (Decoder, field, int)


sumDecoder : Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (field "x" int)
        (field "y" int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 Decoder
-- DELETE Decoder, field, int
-- END SIMULATION
-- === expected output below ===
-- module Math exposing (..)
--
-- import Json.Decode
--
--
-- sumDecoder : Json.Decode.Decoder Int
-- sumDecoder =
--     Json.Decode.map2 (+)
--         (Json.Decode.field "x" Json.Decode.int)
--         (Json.Decode.field "y" Json.Decode.int)
