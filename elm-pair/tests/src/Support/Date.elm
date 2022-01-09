module Support.Date exposing (Clock(..), Date, Hours, Minutes, Time, now)


type Clock
    = Watch
    | Alarm
    | Grandfather


type Date
    = Yesterday
    | Today
    | Tomorrow


type alias Time =
    { hours : Hours
    , minutes : Minutes
    }


type alias Hours =
    Int


type alias Minutes =
    Int


now : Date
now =
    Today
