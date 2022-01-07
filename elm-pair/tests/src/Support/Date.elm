module Support.Date exposing (Clock(..), Date, Time, now)


type Clock
    = Watch
    | Alarm
    | Grandfather


type Date
    = Yesterday
    | Today
    | Tomorrow


type alias Time =
    { hours : Int
    , minutes : Int
    }


now : Date
now =
    Today
