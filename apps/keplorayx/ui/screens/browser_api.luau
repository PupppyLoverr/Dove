--!strict
-- A browser page's host side and what an address typed becomes, ported
-- from Zeron's screens/panel/browser_api (MIT; see THIRD_PARTY_NOTICES.md),
-- itself the shipping app's rules (normalize_address). A refusal is said in
-- words, not a message key.
--
--   api.web(id)                 zgx's page for `id` (ui.web), kept by the host
--   api.view(props)             zgx's web view (ui.webview)
--   api.resolve(text, engine)   where typed text goes: an address, or a search;
--                               nil and why for a refused one
--   api.allowed(url), api.host_of(url), api.open_externally(url)

local M = {}

function M.web(id: string, options: any?): any
	if options then
		return (ui :: any).web(id, options)
	end
	return (ui :: any).web(id)
end

function M.view(props: any): Node
	return (ui :: any).webview(props)
end

function M.open_externally(url: string)
	if M.allowed(url) then
		pcall(ui.invoke, "open_url", url)
	end
end

local LOCAL_FILE = "That is a file on this computer, not a web address"

-- A filesystem path or `file:` URL typed as an address (the shipping app's
-- #962: left to a URL parser, `/Users/me/page.html` became a website).
local function local_file(text: string): boolean
	local drive = string.match(text, "^%a:[/\\]") ~= nil
	return (string.sub(text, 1, 1) == "/" and string.sub(text, 1, 2) ~= "//")
		or string.sub(text, 1, 1) == "\\"
		or text == "~"
		or string.sub(text, 1, 2) == "~/"
		or string.sub(text, 1, 2) == "./"
		or string.sub(text, 1, 3) == "../"
		or drive
end

local function loopback(host: string): boolean
	if host == "localhost" or string.match(host, "%.localhost$") then
		return true
	end
	if string.match(host, "^127%.%d+%.%d+%.%d+$") then
		return true
	end
	return host == "::1" or host == "0:0:0:0:0:0:0:1"
end

-- The authority of `rest` (what follows `scheme://`): up to its path, query
-- or fragment; and its host and port.
local function authority(rest: string): (string, string?, string?)
	local auth = string.match(rest, "^([^/?#]*)") or ""
	local host = string.match(auth, "^%[([^%]]+)%]") or string.match(auth, "^([^:]*)")
	local port = string.match(auth, "%]:(%d+)$") or (if string.match(auth, "^%[") then nil else string.match(auth, ":(%d+)$"))
	return auth, host, port
end

function M.normalize(input: string): (string?, string?)
	local text = string.match(input, "^%s*(.-)%s*$") or ""
	if text == "" then
		return nil, "Type an address"
	end
	if string.find(text, "%c") then
		return nil, "That address has characters an address can't"
	end
	if local_file(text) then
		return nil, LOCAL_FILE
	end
	-- A bare host:port looks like a URI scheme to a parser: only a numeric
	-- port is read that way.
	local head = string.match(text, "^([^/?#]*)") or text
	local host_port = string.match(head, ":(%d+)$") ~= nil
	local explicit = string.find(text, "://", 1, true) ~= nil
		or (string.find(text, ":", 1, true) ~= nil and not host_port and string.sub(text, 1, 1) ~= "[")
	local url = if explicit then text else `https://{text}`
	local scheme, rest = string.match(url, "^(%a[%w+.-]*)://(.*)$")
	if not scheme or not rest then
		return nil, "That isn't an address"
	end
	scheme = string.lower(scheme)
	if scheme ~= "http" and scheme ~= "https" then
		return nil, "Only http and https pages open here"
	end
	local auth, host, port = authority(rest)
	if string.find(auth, "@", 1, true) then
		return nil, "An address with a password in it isn't opened"
	end
	if not host or host == "" or string.find(host, "%s") then
		return nil, "That isn't an address"
	end
	host = string.lower(host)
	if not explicit and loopback(host) then
		scheme = "http"
	end
	local path = string.sub(rest, #auth + 1)
	if path == "" or string.sub(path, 1, 1) ~= "/" then
		path = `/{path}`
	end
	local shown_host = if string.find(host, ":", 1, true) then `[{host}]` else host
	return `{scheme}://{shown_host}{if port then `:{port}` else ""}{path}`, nil
end

-- What typed text should go to: an address (as `normalize` makes it), or a
-- search when it doesn't read as one (words with spaces, a single word with
-- no dot, port or scheme: "react hooks", "zeron"), on `engine` (its address
-- with `%s` where the words go). An address `normalize` refuses for being
-- a file, credentials or another scheme stays refused, as there's no
-- search a path or a `javascript:` should become.
local KEEP_REFUSED = {
	[LOCAL_FILE] = true,
	["An address with a password in it isn't opened"] = true,
	["Only http and https pages open here"] = true,
	["That address has characters an address can't"] = true,
	["Type an address"] = true,
}

local function percent_encode(text: string): string
	return (string.gsub(text, "[^%w%-%._~]", function(c: string): string
		if c == " " then
			return "+"
		end
		return string.format("%%%02X", string.byte(c))
	end))
end

function M.search_url(words: string, engine: string): string
	local q = percent_encode(string.match(words, "^%s*(.-)%s*$") or words)
	return (string.gsub(engine, "%%s", function(): string
		return q
	end))
end

local function reads_as_address(text: string): boolean
	if string.find(text, "%s") then
		return false
	end
	if string.find(text, "://", 1, true) then
		return true
	end
	local head = string.match(text, "^([^/?#]*)") or text
	-- A host with a port, a bracketed IP, a dot inside it, or a loopback
	-- name.
	if string.match(head, ":%d+$") or string.sub(head, 1, 1) == "[" or string.match(head, "^[^.]+%.[^.]") then
		return true
	end
	local host = string.lower(string.match(head, "^([^:]*)") or head)
	return host == "localhost" or string.match(host, "%.localhost$") ~= nil
end

function M.resolve(input: string, engine: string?): (string?, string?, boolean?)
	local text = string.match(input, "^%s*(.-)%s*$") or ""
	local url, err = M.normalize(text)
	if url and reads_as_address(text) then
		return url, nil, false
	end
	if err and KEEP_REFUSED[err] then
		return nil, err, nil
	end
	if not engine or engine == "" then
		return url, err, false
	end
	return M.search_url(text, engine), nil, true
end

function M.allowed(url: string): boolean
	local scheme, rest = string.match(url, "^(%a[%w+.-]*)://(.*)$")
	if not scheme or not rest then
		return false
	end
	scheme = string.lower(scheme)
	if scheme ~= "http" and scheme ~= "https" then
		return false
	end
	local auth, host = authority(rest)
	return host ~= nil and host ~= "" and string.find(auth, "@", 1, true) == nil
end

function M.host_of(url: string?): string?
	if not url then
		return nil
	end
	local rest = string.match(url, "^%a[%w+.-]*://(.*)$")
	if not rest then
		return nil
	end
	local _, host = authority(rest)
	return if host and host ~= "" then host else nil
end

return M
