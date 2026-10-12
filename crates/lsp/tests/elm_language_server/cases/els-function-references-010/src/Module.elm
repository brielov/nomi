foo =
--^
    { field : String }

test =
    { foo | field = "" }
     --X

