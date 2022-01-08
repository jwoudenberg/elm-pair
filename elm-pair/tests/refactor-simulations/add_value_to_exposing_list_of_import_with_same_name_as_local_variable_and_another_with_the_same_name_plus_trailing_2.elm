module Main exposing (..)

import Json.Decode exposing (Decoder, int)


incDecoder : Decoder Int
incDecoder =
    let
        map =
            42
    in
    Json.Decode.map (\n -> n + 1) int



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 )
-- INSERT , map, map2
-- END SIMULATION
-- === expected output below ===
-- module Main exposing (..)
--
-- import Json.Decode exposing (Decoder, int, map, map2)
--
--
-- incDecoder : Decoder Int
-- incDecoder =
--     let
--         map2 =
--             42
--     in
--     map (\n -> n + 1) int
