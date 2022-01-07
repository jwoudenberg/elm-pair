module Main exposing (..)

import Json.Decode


sumDecoder : Json.Decode.Decoder Int
sumDecoder =
    Json.Decode.map2 (+)
        (Json.Decode.field "x" Json.Decode.int)
        (Json.Decode.field "y" Json.Decode.int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 Decode
-- DELETE Decode
-- INSERT Decode exposing (..)
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (..)
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     map2 (+)
--         (field "x" int)
--         (field "y" int)
