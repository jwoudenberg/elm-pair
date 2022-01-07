module Main exposing (..)


type Clock
    = Watch
    | Alarm
    | Grandfather


type alias WholeNumber =
    Int


hindsight : Float
hindsight =
    20 / 20



-- === expected output below ===
-- Type { name: "Clock", constructors: ["Watch", "Alarm", "Grandfather"] }
-- TypeAlias { name: "WholeNumber" }
-- Value { name: "hindsight" }
