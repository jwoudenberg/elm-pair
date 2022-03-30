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
-- INSERT Invalid.Alias
-- END SIMULATION
-- === expected output below ===
-- No refactor for this change.
