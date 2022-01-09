module Main exposing (..)


type Clock
    = Watch
    | Alarm
    | Grandfather


type alias WholeNumber =
    Int


type alias Point =
    { x : Int, y : Int }


hindsight : Float
hindsight =
    20 / 20



-- === expected output below ===
-- Type { name: "Clock", constructors: ["Watch", "Alarm", "Grandfather"] }
-- Type { name: "WholeNumber", constructors: [] }
-- RecordTypeAlias { name: "Point" }
-- Value { name: "hindsight" }
