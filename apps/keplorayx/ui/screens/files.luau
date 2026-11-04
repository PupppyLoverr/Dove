--!strict
-- Files: the project's folder as a tree, each file marked as git sees it,
-- and the one you pick shown below, as Zeron's files pane does (ds/
-- files_pane, file_viewer; MIT, see THIRD_PARTY_NOTICES.md). Code is
-- highlighted by kx/syntax with its lines numbered, read-only and
-- selectable; a picture shows itself; an Office document shows its words,
-- and any document opens in its own app.

local kx = require("@kx")
local store = require("@kx/store")
local Syntax = require("@kx/syntax")

local M = {}

type Item = { name: string, path: string, dir: boolean, size: number, status: string }
type Doc = { path: string, text: string, lang: string, size: number, binary: boolean, truncated: boolean, kind: string, full: string }

-- Folders read so far, by path ("" the root), and which are open.
local listed = ui.state("files.listed", {} :: { [string]: { Item } })
local expanded = ui.state("files.expanded", { [""] = true } :: { [string]: boolean })
local selected = ui.state("files.selected", "")
local doc = ui.state("files.doc", nil :: Doc?)
local failure = ui.state("files.failure", "")
local filter = ui.state("files.filter", "")

local function load(path: string)
	store.list_dir(nil, path):after(function(items: any)
		local next = table.clone(listed:peek())
		next[path] = items or {}
		listed:set(next)
	end, function(why: any)
		failure:set(tostring(if type(why) == "table" then why.message else why))
	end)
end

local function toggle(item: Item)
	local next = table.clone(expanded:peek())
	if next[item.path] then
		next[item.path] = nil
	else
		next[item.path] = true
		if not listed:peek()[item.path] then
			load(item.path)
		end
	end
	expanded:set(next)
end

local function open(item: Item)
	selected:set(item.path)
	store.read_file(nil, item.path):after(function(d: any)
		local given = (d or {}) :: any
		given.path = item.path
		doc:set(given)
	end, function(why: any)
		doc:set(nil)
		failure:set(tostring(if type(why) == "table" then why.message else why))
	end)
end

-- Every visible row: the tree walked depth-first through the open folders,
-- or, filtered, every listed file whose path holds the words.
type Row = { item: Item, depth: number }
local rows = ui.computed(function(): { Row }
	local out: { Row } = {}
	local all, open_ = listed:get(), expanded:get()
	local q = string.lower(filter:get())
	if q ~= "" then
		for _, items in all do
			for _, it in items do
				if not it.dir and string.find(string.lower(it.path), q, 1, true) then
					table.insert(out, { item = it, depth = 0 })
				end
			end
		end
		table.sort(out, function(a, b)
			return a.item.path < b.item.path
		end)
		return out
	end
	local function walk(path: string, depth: number)
		for _, it in all[path] or {} do
			table.insert(out, { item = it, depth = depth })
			if it.dir and open_[it.path] then
				walk(it.path, depth + 1)
			end
		end
	end
	walk("", 0)
	return out
end)

local MARK = { M = "modified", A = "added", ["?"] = "untracked", D = "deleted", R = "modified" }

local function icon_for(it: Item): string
	if it.dir then
		return "folder"
	end
	local ext = string.lower(string.match(it.name, "%.([^.]+)$") or "")
	if ext == "md" or ext == "markdown" then
		return "file-markdown"
	elseif ext == "png" or ext == "jpg" or ext == "jpeg" or ext == "gif" or ext == "webp" or ext == "svg" then
		return "file-image"
	elseif ext == "json" or ext == "toml" or ext == "yaml" or ext == "yml" or ext == "csv" then
		return "file-data"
	elseif ext == "css" or ext == "scss" then
		return "file-style"
	elseif Syntax.language(it.name) ~= nil then
		return "file-code"
	end
	return "document"
end

local function row_view(r: Row): Node
	local it = r.item
	return kx.Row {
		size = "sm",
		pl = ui.raw(8 + r.depth * 14, "a tree's indent, by depth"),
		label = it.path,
		selected = function()
			return selected:get() == it.path
		end,
		on_click = function()
			if it.dir then
				toggle(it)
			else
				open(it)
			end
		end,
		ui.column {
			w = 12,
			color = "$text.tertiary",
			if it.dir
				then ui.column {
					rotate = function(): number
						return if expanded:get()[it.path] then 0 else -90
					end,
					kx.Icon { name = "chevron-down", size = 11 },
				}
				else nil,
		},
		ui.column { color = "$text.secondary", kx.Icon { name = icon_for(it), size = 15 } },
		ui.text {
			if filter:peek() ~= "" then it.path else it.name,
			literal = true,
			truncate = true,
			grow = 1,
			min_w = 0,
			text_size = "$font.size.md",
			color = if it.status ~= "" then `$kx.git.{MARK[it.status] or "modified"}` else "$text.primary",
		},
		if it.status ~= ""
			then ui.text {
				if it.status == "?" then "U" else it.status,
				literal = true,
				text_size = "$font.size.xs",
				weight = "$font.weight.medium",
				color = `$kx.git.{MARK[it.status] or "modified"}`,
			}
			else nil,
	}
end

local function size_text(n: number): string
	if n < 1024 then
		return `{n} B`
	elseif n < 1024 * 1024 then
		return string.format("%.1f KB", n / 1024)
	end
	return string.format("%.1f MB", n / 1024 / 1024)
end

-- The file picked, as its kind shows best.
local function viewer(): Node
	return ui.presence(function()
		local d = doc:get()
		return if d then `{d.path}:{d.kind}` else nil
	end, function(_key: string)
		local d = doc:peek() :: Doc
		local header = ui.row {
			w_full = true,
			h = 32,
			px = "$space.3",
			gap = "$space.2",
			items = "center",
			border_sides = { 1, 0, 1, 0 },
			border_color = "$border.subtle",
			kx.Mono { d.path, literal = true, truncate = true, grow = 1, min_w = 0, text_size = "$font.size.sm" },
			kx.Hint { size_text(d.size), literal = true },
			kx.Button {
				tone = "ghost",
				size = "xs",
				icon = true,
				label = "Open in its app",
				tooltip = "Open in its app",
				on_click = function()
					pcall(ui.invoke, "open_url", d.full)
				end,
				kx.Icon { name = "arrow-up-right", size = 13 },
			},
		}
		local body: Node
		if d.kind == "image" then
			body = ui.column {
				grow = 1,
				min_h = 0,
				w_full = true,
				p = "$space.4",
				items = "center",
				justify = "center",
				ui.image { src = d.full, max_w = "100%", max_h = "100%", fit = "contain", rounded = "$radius.md" },
			}
		elseif d.kind == "binary" or (d.kind == "document" and d.text == "") then
			body = kx.Empty {
				icon = "document",
				title = if d.kind == "binary" then "Not text" else "A document",
				hint = "Open it in its own app to see it.",
				action = kx.Button {
					tone = "secondary",
					size = "sm",
					on_click = function()
						pcall(ui.invoke, "open_url", d.full)
					end,
					"Open",
				},
			}
		else
			local lang = if d.kind == "document" then nil else Syntax.language(d.path)
			body = ui.textarea {
				value = d.text,
				read_only = true,
				line_numbers = d.kind ~= "document",
				wrap = d.kind == "document" or lang == "markdown",
				highlight = if lang then Syntax.highlighter(lang) else nil,
				grow = 1,
				min_h = 0,
				w_full = true,
				p = "$space.3",
				bg = "#00000000",
				border = 0,
				focus = { border = 0, outline = 0 },
				font = if d.kind == "document" then "$font.sans" else "$font.mono",
				text_size = "$font.size.sm",
				line_height = "$font.line.md",
				color = "$kx.syntax.text",
				label = d.path,
			}
		end
		return ui.column {
			grow = 1,
			min_h = 0,
			w_full = true,
			header,
			body,
			if d.truncated then kx.Hint { "Shown in part: the file is long.", literal = true, p = "$space.2" } else nil,
		}
	end, { grow = 1, min_h = 0, w_full = true })
end

function M.view(): Node
	-- Each time the panel shows Files, the root is read again.
	load("")
	for path in expanded:peek() do
		if path ~= "" then
			load(path)
		end
	end
	return ui.column {
		grow = 1,
		min_h = 0,
		w_full = true,
		ui.row {
			w_full = true,
			px = "$space.2",
			pb = "$space.2",
			kx.Field {
				value = filter,
				placeholder = "Find a file",
				label = "Find a file",
				grow = 1,
				min_w = 0,
				h = 28,
			},
		},
		ui.show(function()
			return failure:get() ~= ""
		end, kx.Hint {
			function()
				return failure:get()
			end,
			literal = true,
			px = "$space.3",
			color = "$danger",
		}),
		ui.scroll {
			w_full = true,
			-- A file open: the tree keeps a third, the file the rest.
			grow = function(): number
				return if doc:get() then 0 else 1
			end,
			h = function(): any
				return if doc:get() then 240 else nil
			end,
			min_h = 0,
			px = "$space.1",
			ui.each(rows, row_view, {
				key = function(r: Row)
					return r.item.path
				end,
				w_full = true,
			}),
		},
		ui.show(function()
			return doc:get() ~= nil
		end, viewer()),
	}
end

return M
