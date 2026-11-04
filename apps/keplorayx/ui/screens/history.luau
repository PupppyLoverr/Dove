--!strict
-- History: the project's last commits, newest first, on their graph. Each
-- row draws its slice of the graph (dove-git lays it out: the lanes that
-- pass through, the one that joins into the commit's node, the one that
-- forks from it), then the subject, its branches and tags, who and when.
-- The checked-out commit's node is filled.

local kx = require("@kx")
local store = require("@kx/store")

local M = {}

type Commit = {
	sha: string,
	short: string,
	subject: string,
	author: string,
	when: string,
	refs: { string },
	head: boolean,
	lane: number,
	top: { number },
	bottom: { number },
	joins: { number },
	forks: { number },
}

local commits = ui.state("history.commits", {} :: { Commit })
local failure = ui.state("history.failure", "")
local query = ui.state("history.query", "")
local loading = ui.state("history.loading", false)

local ROW, LANE, PAD, NODE = 30, 12, 10, 3.5

local function lane_x(n: number): number
	return PAD + n * LANE
end

local function load()
	loading:set(true)
	store.history(nil):after(function(list: any)
		commits:set(list or {})
		failure:set("")
		loading:set(false)
	end, function(why: any)
		failure:set(tostring(if type(why) == "table" then why.message else why))
		loading:set(false)
	end)
end

local function graph(c: Commit): Node
	local lanes = 1
	for _, list in { c.top, c.bottom, c.joins, c.forks, { c.lane } } do
		for _, n in list do
			lanes = math.max(lanes, n + 1)
		end
	end
	return ui.canvas {
		w = PAD * 2 + (lanes - 1) * LANE,
		h = ROW,
		shrink = 0,
		pointer_events = "none",
		draw = function(cv: any, _w: number, h: number)
			local mid = h / 2
			local x = lane_x(c.lane)
			for _, n in c.top do
				cv:stroke(ui.path():move(lane_x(n), 0):line(lane_x(n), mid), "$kx.tree", 1.5)
			end
			for _, n in c.bottom do
				cv:stroke(ui.path():move(lane_x(n), mid):line(lane_x(n), h), "$kx.tree", 1.5)
			end
			for _, n in c.joins do
				cv:stroke(ui.path():move(lane_x(n), 0):quad(lane_x(n), mid, x, mid), "$kx.tree", 1.5)
			end
			for _, n in c.forks do
				cv:stroke(ui.path():move(x, mid):quad(lane_x(n), mid, lane_x(n), h), "$kx.tree", 1.5)
			end
			-- The node: filled at HEAD, else a ring (a disc with the ground
			-- laid in it).
			if c.head then
				cv:circle(x, mid, NODE + 0.5, "$accent")
			else
				cv:circle(x, mid, NODE + 0.5, "$text.secondary")
				cv:circle(x, mid, NODE - 1, "$kx.solid")
			end
		end,
	}
end

local function ref_pill(name: string): Node
	local tag = string.sub(name, 1, 5) == "tag: "
	local head = string.find(name, "HEAD", 1, true) ~= nil
	return ui.row {
		h = 18,
		px = "$kx.space.tight",
		gap = "$kx.space.hair",
		items = "center",
		rounded = "$radius.sm",
		shrink = 0,
		bg = if head then "$accent" else "$kx.secondary.bg",
		color = if head then "$accent.text" else "$text.secondary",
		kx.Icon { name = if tag then "tag" else "git-branch", size = 11 },
		ui.text {
			string.gsub(string.gsub(name, "^tag: ", ""), "^HEAD %-> ", ""),
			literal = true,
			text_size = "$font.size.xs",
			weight = "$font.weight.medium",
			truncate = true,
			max_w = 140,
		},
	}
end

local function row(c: Commit): Node
	local refs: { Node } = {}
	for _, r in c.refs do
		table.insert(refs, ref_pill(r))
	end
	return ui.button {
		w_full = true,
		h = ROW,
		items = "center",
		gap = "$space.2",
		pr = "$space.3",
		rounded = "$radius.md",
		cursor = "pointer",
		hover = { bg = "$hover" },
		label = c.subject,
		tooltip = `{c.short} · {c.author} · {c.when}\nClick to copy the commit's id`,
		on_click = function()
			ui.clipboard.write(c.sha)
			kx.toast { title = `Copied {c.short}` }
		end,
		graph(c),
		ui.row { gap = "$kx.space.hair", items = "center", shrink = 0, table.unpack(refs) },
		ui.text {
			c.subject,
			literal = true,
			truncate = true,
			grow = 1,
			min_w = 0,
			text_size = "$font.size.md",
			weight = if c.head then "$font.weight.medium" else "$font.weight.regular",
		},
		ui.text {
			c.when,
			literal = true,
			shrink = 0,
			text_size = "$font.size.xs",
			color = "$text.tertiary",
		},
	}
end

function M.view(): Node
	load()
	local shown = ui.computed(function(): { Commit }
		local q = string.lower(query:get())
		if q == "" then
			return commits:get()
		end
		local out: { Commit } = {}
		for _, c in commits:get() do
			if string.find(string.lower(c.subject .. " " .. c.author .. " " .. c.short), q, 1, true) then
				table.insert(out, c)
			end
		end
		return out
	end)
	return ui.column {
		grow = 1,
		min_h = 0,
		w_full = true,
		ui.row {
			w_full = true,
			px = "$space.2",
			pb = "$space.2",
			gap = "$space.1",
			items = "center",
			kx.Field {
				value = query,
				placeholder = "Find a commit",
				label = "Find a commit",
				grow = 1,
				h = 28,
			},
			kx.Button {
				tone = "ghost",
				size = "sm",
				icon = true,
				label = "Read the history again",
				tooltip = "Read again",
				busy = loading,
				on_click = load,
				kx.Icon { name = "refresh", size = 14 },
			},
		},
		ui.show(
			function()
				return failure:get() ~= ""
			end,
			kx.Empty {
				icon = "git-branch",
				title = "No history to show",
				hint = failure:peek(),
			},
			ui.show(
				function()
					return #commits:get() == 0 and not loading:get()
				end,
				kx.Empty {
					icon = "git-branch",
					title = "No commits yet",
					hint = "This project isn't a git repository, or nothing is committed.",
				},
				ui.scroll {
					grow = 1,
					min_h = 0,
					w_full = true,
					px = "$space.1",
					ui.each(shown, row, {
						key = function(c: Commit)
							return c.sha
						end,
						w_full = true,
					}),
				}
			)
		),
	}
end

return M
