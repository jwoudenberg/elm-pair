module Beverage exposing (..)

import Json.Decode as Dec exposing (Decoder, float)


type alias Beverage =
    { kind : String
    , liters : Float
    }


decode : Decoder Beverage
decode =
    Dec.map2 Beverage Dec.string float



-- START SIMULATION
-- MOVE CURSOR TO LINE 3 Decoder
-- DELETE Decoder, float
-- INSERT ..
-- END SIMULATION
-- === expected output below ===
-- module Beverage exposing (..)
--
-- import Json.Decode as Dec exposing (..)
--
--
-- type alias Beverage =
--     { kind : String
--     , liters : Float
--     }
--
--
-- decode : Decoder Beverage
-- decode =
--     map2 Beverage string float
