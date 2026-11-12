-- The panel's panes: a session's tabs split into groups laid out side by
-- side and above one another (the tree of screens/shell/split), each with a
-- strip of its own and the tab it shows. Kept across launches, by the
-- panel's key (screens/panel/state's), beside the tabs themselves, which
-- stay state's: a group is only which of them it holds.
--
-- The group "main" holds every tab no other group does. A tab opened,
-- whatever opens it, lands in the focused group; one turned to focuses its
-- group. A group whose last tab goes is gone, its neighbours taking its
-- room; with one left, the panel is as it always was. The group "@page"
-- is in no pane: its tabs are out in the page's split, each a pane among
-- the chats (screens/shell/page), and come back as their pane closes.
--
--   groups.split_now(key)          whether the panel shows more than one pane
--   groups.list(key)               its panes' groups, in reading order
--   groups.tabs_in(key, gid)       a group's tabs, in the strip's order
--   groups.active_in(key, gid)     the tab a group shows (nil: its empty page)
--   groups.focused(key)            the focused group
--   groups.shown(key, id)          whether a tab is the one its group shows
--   groups.frames(key), groups.dividers(key)   where each pane stands
--   groups.focus(key, gid)         that group focused
--   groups.move(key, id, gid)      a tab into a group, shown there
--   groups.split(key, id, target, side)  a tab into a new group at
--                                  `target`'s side: false if there's no room
--   groups.resize(key, …)          a divider dragged (as split.resize)
--   groups.to_page(key, id)        a tab out to the page's split: its leaf
--   groups.dragging                a tab being dragged: the panes show
--                                  where it can land
--   groups.MAX                     the most panes in a panel

local state = require("@app/screens/panel/state")
local splits = require("@app/screens/shell/split")

local tree = splits.tree

local M = {}

M.MAIN = "main"
M.PAGE = "@page"
M.MAX = 6
-- The gap between two panes (the window's, as between two chats).
M.GAP = splits.GAP

-- `tabs`: each group's tabs but main's, by id. `actives`: the tab each shows
-- (false: its empty page, asked for). `next`: the next group's number.
export type Layout = { tree: any, tabs: { [string]: { string } }, actives: { [string]: any }, focus: string, next: number }

M.layouts = ui.persist("panel_groups", {} :: { [string]: Layout })
M.dragging = ui.shared("panel.dragging", false)
-- Counts the panels made: what shows a tab outside the panel builds it again
-- with each (screens/panel's `tab_body` is the panel showing's).
M.bodies = ui.shared("panel.bodies", 0)

local function fresh(): Layout
	return { tree = tree.leaf(M.MAIN), tabs = {}, actives = {}, focus = M.MAIN, next = 1 }
end

local function copy(l: Layout?): Layout
	if not l then
		return fresh()
	end
	local tabs: { [string]: { string } } = {}
	for g, list in l.tabs do
		tabs[g] = table.clone(list)
	end
	return { tree = l.tree, tabs = tabs, actives = table.clone(l.actives), focus = l.focus, next = l.next }
end

-- A tab's leaf in the page's split: its panel's key and its id.
function M.leaf(key: string, id: string): string
	return `tab:{id}@{key}`
end
-- The panel's key and the tab of a leaf of the page's split, if it's a tab's.
function M.of_leaf(leaf: string): (string?, string?)
	local id, key = string.match(leaf, "^tab:([^@]+)@(.+)$")
	return key, id
end

-- The group a tab is in, in `l` (unlisted: main's, or with no main pane the
-- focused one's).
local function group_in(l: Layout?, id: string): string
	if not l then
		return M.MAIN
	end
	for g, list in l.tabs do
		if table.find(list, id) then
			return g
		end
	end
	return if table.find(tree.leaves(l.tree), M.MAIN) then M.MAIN else l.focus
end

local function drop_from_all(l: Layout, id: string)
	for g, list in l.tabs do
		local at = table.find(list, id)
		if at then
			table.remove(list, at)
		end
	end
end

-- A layout as it must be for the tabs there are: groups with no tab gone,
-- each showing one of its own, the focus on a pane there is and following
-- the tab last turned to. Nil: one pane, no tab out, nothing to keep.
local function tidy(key: string, given: Layout, session: state.Session): Layout?
	local l = copy(given)
	local there: { [string]: boolean } = {}
	for _, t in session.tabs do
		there[t.id] = true
	end
	local listed: { [string]: boolean } = {}
	for g, list in l.tabs do
		local kept: { string } = {}
		for _, id in list do
			-- (A tab out in the page stays out only while its pane is there.)
			if there[id] and not listed[id] and (g ~= M.PAGE or splits.holds(M.leaf(key, id))) then
				table.insert(kept, id)
				listed[id] = true
			end
		end
		l.tabs[g] = kept
	end
	local unlisted = 0
	for _, t in session.tabs do
		if not listed[t.id] then
			unlisted += 1
		end
	end
	-- Panes with no tab left go.
	for _, g in tree.leaves(l.tree) do
		local empty = if g == M.MAIN then unlisted == 0 else #(l.tabs[g] or {}) == 0
		if empty and #tree.leaves(l.tree) > 1 then
			l.tree = tree.without(l.tree, g) or tree.leaf(M.MAIN)
			if g ~= M.MAIN then
				l.tabs[g] = nil
			end
			l.actives[g] = nil
		end
	end
	for g in l.tabs do
		if g ~= M.PAGE and not table.find(tree.leaves(l.tree), g) then
			l.tabs[g] = nil
		end
	end
	local panes = tree.leaves(l.tree)
	if not table.find(panes, l.focus) then
		l.focus = panes[1]
	end
	-- One pane left: main, whatever it was called.
	if #panes == 1 and panes[1] ~= M.MAIN then
		local g = panes[1]
		l.actives[M.MAIN], l.actives[g] = l.actives[g], nil
		l.tabs[g] = nil
		l.tree = tree.leaf(M.MAIN)
		l.focus = M.MAIN
		panes = { M.MAIN }
	elseif not table.find(panes, M.MAIN) and unlisted > 0 then
		-- No main pane: a tab in no group is the focused one's.
		local list = l.tabs[l.focus] or {}
		for _, t in session.tabs do
			if not listed[t.id] then
				table.insert(list, t.id)
			end
		end
		l.tabs[l.focus] = list
	end
	-- The tab last turned to is its group's, and its group focused; none (the
	-- empty page asked for), the focused group's.
	local a = session.active
	if a and there[a] then
		local g = group_in(l, a)
		if g ~= M.PAGE then
			l.focus = g
			l.actives[g] = a
		end
	elseif a == nil then
		l.actives[l.focus] = false
	end
	-- Each other pane shows one of its own: the one it did, else the last
	-- visited of them, else its last.
	for _, g in panes do
		local shows = l.actives[g]
		local mine: { [string]: boolean } = {}
		local last: string? = nil
		for _, t in session.tabs do
			if group_in(l, t.id) == g then
				mine[t.id] = true
				last = t.id
			end
		end
		local keep = (type(shows) == "string" and mine[shows]) or (shows == false and g == l.focus and a == nil)
		if not keep then
			local pick: string? = nil
			for _, seen in session.visited do
				if mine[seen] then
					pick = seen
					break
				end
			end
			l.actives[g] = pick or last
		end
	end
	if #panes == 1 and #(l.tabs[M.PAGE] or {}) == 0 then
		return nil
	end
	if #(l.tabs[M.PAGE] or {}) == 0 then
		l.tabs[M.PAGE] = nil
	end
	return l
end

-- Kept in step with the tabs: as they open, close and are turned to, as a
-- group changes, and as a pane of the page's split goes.
ui.effect(function()
	local sessions = state.sessions:get()
	local all = M.layouts:get()
	local out: any = nil
	for key, l in all do
		-- (A panel whose tabs aren't known yet keeps its panes as they were kept:
		-- nothing is tidied away before there's something to tidy by.)
		local session = sessions[key]
		local now = if session then tidy(key, l, session) else l
		if not tree.same(now, l) then
			out = out or table.clone(all)
			out[key] = now
		end
	end
	if out then
		M.layouts:set(out)
	end
end)

local function write(key: string, l: Layout)
	local all = table.clone(M.layouts:peek())
	all[key] = l
	M.layouts:set(all)
end

function M.of(key: string?): Layout?
	return if key then M.layouts:get()[key] else nil
end

function M.list(key: string?): { string }
	local l = M.of(key)
	return if l then tree.leaves(l.tree) else { M.MAIN }
end

function M.split_now(key: string?): boolean
	local l = M.of(key)
	return l ~= nil and #tree.leaves(l.tree) > 1
end

function M.group_of(key: string, id: string): string
	return group_in(M.layouts:peek()[key], id)
end

function M.tabs_in(key: string?, gid: string): { state.Tab }
	local out: { state.Tab } = {}
	if not key then
		return out
	end
	local l = M.of(key)
	for _, t in state.of(key).tabs do
		if group_in(l, t.id) == gid then
			table.insert(out, t)
		end
	end
	return out
end

function M.focused(key: string?): string
	local l = M.of(key)
	return if l then l.focus else M.MAIN
end

function M.active_in(key: string?, gid: string): string?
	if not key then
		return nil
	end
	local l = M.of(key)
	if not l then
		return state.of(key).active
	end
	local a = l.actives[gid]
	return if type(a) == "string" then a else nil
end

function M.shown(key: string?, id: string): boolean
	if not key then
		return false
	end
	local l = M.of(key)
	if not l then
		return state.of(key).active == id
	end
	return l.actives[group_in(l, id)] == id
end

function M.frames(key: string?): { [string]: any }
	local l = M.of(key)
	return if l then tree.frames(l.tree, M.GAP) else {}
end

function M.dividers(key: string?): { any }
	local l = M.of(key)
	if not l then
		return {}
	end
	local _, list = tree.arrange(l.tree, M.GAP)
	return list
end

function M.focus(key: string, gid: string)
	local l = M.layouts:peek()[key]
	if not l or not table.find(tree.leaves(l.tree), gid) then
		return
	end
	if l.focus == gid then
		return
	end
	-- (The tab it shows turned to, which focuses it; showing none, its empty page.)
	local shows = l.actives[gid]
	local tab: string? = if type(shows) == "string" then shows else nil
	local now = copy(l)
	now.focus = gid
	if not tab then
		now.actives[gid] = false
	end
	ui.batch(function()
		write(key, now)
		state.activate(key, tab)
	end)
end

-- A tab into a group (one there is), shown there and focused.
function M.move(key: string, id: string, gid: string)
	local l = copy(M.layouts:peek()[key])
	if gid ~= M.MAIN and not table.find(tree.leaves(l.tree), gid) then
		return
	end
	if group_in(l, id) ~= gid then
		drop_from_all(l, id)
		if gid ~= M.MAIN then
			local list = l.tabs[gid] or {}
			table.insert(list, id)
			l.tabs[gid] = list
		end
	end
	l.actives[gid] = id
	l.focus = gid
	ui.batch(function()
		write(key, l)
		state.activate(key, id)
	end)
end

-- A tab into a new group, at the side of the pane `target` ("left", "right",
-- "top" or "bottom"). False when the panel has all the panes it can hold.
function M.split(key: string, id: string, target: string, side: string): boolean
	local l = copy(M.layouts:peek()[key])
	local panes = tree.leaves(l.tree)
	if not table.find(panes, target) then
		return false
	end
	-- (The only tab of the pane it's dropped at the side of: nothing to split.)
	local from = group_in(l, id)
	local alone = true
	for _, t in state.of(key).tabs do
		if t.id ~= id and group_in(l, t.id) == from then
			alone = false
		end
	end
	if from == target and alone then
		return true
	end
	-- (One whose own pane goes as it leaves makes no more panes than there were.)
	if #panes >= M.MAX and not (alone and table.find(panes, from)) then
		return false
	end
	local gid = `g{l.next}`
	l.next += 1
	drop_from_all(l, id)
	l.tabs[gid] = { id }
	l.tree = tree.insert(l.tree, target, gid, side) or l.tree
	l.actives[gid] = id
	l.focus = gid
	ui.batch(function()
		write(key, l)
		state.activate(key, id)
	end)
	return true
end

function M.shares_at(key: string, path: { number }): { number }
	local l = M.layouts:peek()[key]
	return tree.shares_in(if l then l.tree else nil, path)
end

function M.resize(key: string, path: { number }, at: number, start: { number }, moved: number, least: number)
	local l = M.layouts:peek()[key]
	if not l then
		return
	end
	local now = copy(l)
	now.tree = tree.resized(l.tree, path, at, start, moved, least)
	write(key, now)
end

-- A tab out to the page's split: in no pane of the panel until its pane
-- there closes. Its leaf, for the split.
function M.to_page(key: string, id: string): string
	local l = copy(M.layouts:peek()[key])
	local from = group_in(l, id)
	if from ~= M.PAGE then
		drop_from_all(l, id)
		local list = l.tabs[M.PAGE] or {}
		table.insert(list, id)
		l.tabs[M.PAGE] = list
		-- (Its group shows another of its own; none left, its empty page.)
		if l.actives[from] == id then
			l.actives[from] = nil
		end
	end
	local s = state.of(key)
	ui.batch(function()
		write(key, l)
		if s.active == id then
			local next_tab: string? = nil
			for _, seen in s.visited do
				if seen ~= id and group_in(l, seen) ~= M.PAGE and state.find(key, seen) then
					next_tab = seen
					break
				end
			end
			state.activate(key, next_tab)
		end
	end)
	return M.leaf(key, id)
end

-- Whether a tab is out in the page's split.
function M.in_page(key: string?, id: string): boolean
	local l = M.of(key)
	return l ~= nil and table.find(l.tabs[M.PAGE] or {}, id) ~= nil
end

return M
