--!strict
-- Home with the composer placed: slim.
local home = require("@fixtures/home")
local f = table.clone(home :: any)
f.composer_place = "slim"
return f
