--!strict
-- Files: Zeron's file pane (ds/files_pane, file_tree, file_viewer, vendored;
-- MIT, see THIRD_PARTY_NOTICES.md). The project's tree at the left, every
-- file marked as git sees it and a filter over them; the one you pick
-- beside it, highlighted, a picture as itself, an Office document as its
-- words; one band across the top of both. Too narrow for the two, the tree
-- floats over the file. Its toggle hides the tree.

local FilesPane = require("@ds/files_pane")
local FileTree = require("@ds/file_tree")
local kx = require("@kx")
local store = require("@kx/store")

local M = {}

local fixture = store.fixture ~= ""
local function kept<T>(key: string, default: T): State<T>
	return if fixture then ui.state(key, default) else ui.persist(key, default)
end

local shown = kept("files.tree_shown", true)
local active = ui.state("files.active", nil :: string?)
local filter = ui.state("files.filter", "")
local doc = ui.state("files.doc", nil :: any)
-- What was listed, as the tree reads it, and how many times (a pane built
-- again takes the model as it stands).
type Listing = { name: string, root: string, files: string, statuses: { [string]: string } }
local listing = ui.state("files.listing", nil :: Listing?)
local failure = ui.state("files.failure", "")

local function load()
	store.list_files(nil):after(function(l: any)
		listing:set(l)
		failure:set("")
	end, function(why: any)
		failure:set(tostring(if type(why) == "table" then why.message else why))
	end)
end

-- A file read for the viewer, as its doc.
local function open(path: string)
	active:set(path)
	doc:set({ path = path, state = "loading" })
	store.read_file(nil, path):after(function(d: any)
		local status = (listing:peek() or {} :: any).statuses
		local mark = if status then status[path] else nil
		local k = d.kind
		if k == "image" then
			doc:set({ path = path, state = "ready", image = d.full, size = d.size, status = mark, absolute = d.full })
		elseif k == "binary" or (k == "document" and d.text == "") then
			doc:set({ path = path, state = "binary", size = d.size, status = mark, absolute = d.full })
		else
			doc:set({ path = path, state = "ready", text = d.text, size = d.size, status = mark, absolute = d.full })
		end
	end, function(why: any)
		doc:set({ path = path, state = "error", error = tostring(if type(why) == "table" then why.message else why) })
	end)
end

local function pane(l: Listing): Node
	local model = FileTree.lines(l.name, l.files, l.root)
	FileTree.statuses(model, l.statuses or {})
	return FilesPane {
		shown = shown,
		tree = {
			model = model,
			active = active,
			filter = filter,
			search = true,
			on_open = open,
		},
		viewer = {
			doc = doc,
			root = l.root,
			on_reveal = function(path: string)
				active:set(path)
			end,
			on_open_default = function(path: string)
				pcall(ui.invoke, "open_url", `{l.root}\\{string.gsub(path, "/", "\\")}`)
			end,
		},
		grow = 1,
		min_h = 0,
		w_full = true,
	}
end

function M.view(): Node
	-- Each time the panel shows Files, the project is listed again.
	load()
	return ui.column {
		grow = 1,
		min_h = 0,
		w_full = true,
		ui.show(
			function()
				return failure:get() ~= ""
			end,
			kx.Empty {
				icon = "folder-with-files",
				title = "No files to show",
				hint = function()
					return failure:get()
				end,
			},
			ui.presence(function(): string?
				local l = listing:get()
				-- (Built again only as the list itself changes.)
				return if l then `{l.root}:{#l.files}` else nil
			end, function(_key: string): Node
				return pane(listing:peek() :: Listing)
			end, { grow = 1, min_h = 0, w_full = true })
		),
	}
end

return M
