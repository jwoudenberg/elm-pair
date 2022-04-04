module ModuleNotExposingVariable exposing (whitespace)


greeting : String
greeting =
    "Hi"


whitespace : String
whitespace =
    "   "



-- === expected output below ===
-- module ModuleNotExposingVariable exposing (whitespace)
--
--
-- greetz : String
-- greetz =
--     "Hi"
--
--
-- whitespace : String
-- whitespace =
--     "   "
