module Main exposing (..)

import Json.Decode exposing (int)


type Decoder
    = Xml
    | Json
    | Morse


decoderDecoder : Json.Decode.Decoder Decoder
decoderDecoder =
    Json.Decode.map
        (\n ->
            case n of
                0 ->
                    Xml

                1 ->
                    Json

                _ ->
                    Morse
        )
        int



-- START SIMULATION
-- MOVE CURSOR TO LINE 12 Json.Decode.Decoder
-- DELETE Json.Decode.
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, int)
--
--
-- type Decoder2
--     = Xml
--     | Json
--     | Morse
--
--
-- decoderDecoder : Decoder Decoder2
-- decoderDecoder =
--     Json.Decode.map
--         (\n ->
--             case n of
--                 0 ->
--                     Xml
--
--                 1 ->
--                     Json
--
--                 _ ->
--                     Morse
--         )
--         int
