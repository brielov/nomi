
type User = User { data : String }
--X

func: User
    --^
func =
User { data = "" }
