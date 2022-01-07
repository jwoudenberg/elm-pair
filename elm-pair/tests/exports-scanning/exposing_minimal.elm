module Main exposing (minimal)


minimal : ()
minimal =
    ()


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
-- Value { name: "minimal" }
