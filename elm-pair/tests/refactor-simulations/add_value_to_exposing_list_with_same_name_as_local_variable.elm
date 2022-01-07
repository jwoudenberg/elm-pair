module Main exposing (..)

import Json.Decode exposing (Decoder, int)


sumDecoder : Decoder Int
sumDecoder =
    let
        field =
            "x"

        field2 =
            "y"
    in
    Json.Decode.map2 (+)
        (Json.Decode.field field int)
        (Json.Decode.field field2 int)



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 , int
-- INSERT , field
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, field, int)
--
--
-- sumDecoder : Decoder Int
-- sumDecoder =
--     let
--         field3 =
--             "x"
--
--         field2 =
--             "y"
--     in
--     Json.Decode.map2 (+)
--         (field field3 int)
--         (field field2 int)
