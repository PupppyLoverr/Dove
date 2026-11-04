--!strict
-- Team: who works with you. Your agents first, each in its own light and
-- whether it's on this device; then your teammates (Dove's personas): a
-- name, a role, the agent they prefer and how they work, as cards in the
-- manner of Zeron's member cards (ds/member_card; MIT, see
-- THIRD_PARTY_NOTICES.md). A teammate starts a thread as itself, and its
-- brief leads that thread's first turn.

local kx = require("@kx")
local store = require("@kx/store")
local nav = require("@screens/nav")

local M = {}

type Agent = { id: string, label: string, installed: boolean }
type Member = { id: string, name: string, role: string, avatar: string, hue: number, harness: string, instructions: string }

local agents = ui.state("team.agents", {} :: { Agent })
local members = ui.state("team.members", {} :: { Member })
local editing = ui.state("team.editing", nil :: Member?)

local function load()
	store.team():after(function(t: any)
		agents:set((t and t.agents) or {})
		members:set((t and t.members) or {})
	end, function() end)
end

local function heading(title: string, hint: string): Node
	return ui.column {
		w_full = true,
		gap = "$kx.space.hair",
		pt = "$space.5",
		pb = "$space.2",
		ui.text { title, literal = true, text_size = "$font.size.lg", weight = "$font.weight.medium" },
		kx.Caption { hint, literal = true, wrap = true },
	}
end

local function agent_card(a: Agent): Node
	return ui.row {
		w = 220,
		h = 56,
		px = "$space.3",
		gap = "$space.3",
		items = "center",
		rounded = "$radius.xl",
		bg = "$surface.base",
		inset_shadow = "$kx.lip.inset",
		opacity = if a.installed then 1 else 0.6,
		kx.AgentMark { agent = a.id, size = 20 },
		ui.column {
			grow = 1,
			min_w = 0,
			ui.text { a.label, literal = true, truncate = true, weight = "$font.weight.medium" },
			kx.Hint { if a.installed then "On this device" else "Not installed", literal = true },
		},
		if a.installed then kx.StatusIcon { kind = "done", size = 14 } else nil,
	}
end

local function avatar(m: Member, size: number): Node
	return ui.row {
		w = size,
		h = size,
		shrink = 0,
		rounded = "$radius.lg",
		bg = "$kx.secondary.bg",
		items = "center",
		justify = "center",
		ui.text {
			if m.avatar ~= "" then m.avatar else string.upper(string.sub(m.name, 1, 1)),
			literal = true,
			text_size = "$font.size.lg",
			weight = "$font.weight.semibold",
			color = kx.tokens.agent(if m.harness ~= "" then m.harness else nil),
		},
	}
end

local function start_as(m: Member)
	local ray = require("@screens/ray")
	ray.persona:set({ id = m.id, name = m.name })
	if m.harness ~= "" then
		ray.agent:set(m.harness)
	end
	nav.go("home")
	store.home()
	nav.focus_ray()
end

local function member_card(m: Member): Node
	return ui.column {
		w = 300,
		p = "$space.4",
		gap = "$space.3",
		rounded = "$radius.xl",
		bg = "$surface.base",
		inset_shadow = "$kx.lip.inset",
		ui.row {
			w_full = true,
			gap = "$space.3",
			items = "center",
			avatar(m, 40),
			ui.column {
				grow = 1,
				min_w = 0,
				ui.text { m.name, literal = true, truncate = true, text_size = "$font.size.lg", weight = "$font.weight.medium" },
				kx.Hint { if m.role ~= "" then m.role else "Teammate", literal = true, truncate = true },
			},
		},
		if m.instructions ~= ""
			then kx.Caption { m.instructions, literal = true, wrap = true, lines = 3 }
			else nil,
		ui.row {
			w_full = true,
			gap = "$space.1",
			items = "center",
			kx.Button {
				size = "sm",
				on_click = function()
					start_as(m)
				end,
				"Start a thread",
			},
			ui.div { grow = 1 },
			kx.Button {
				tone = "ghost",
				size = "sm",
				icon = true,
				label = "Edit",
				tooltip = "Edit",
				on_click = function()
					editing:set(table.clone(m))
				end,
				kx.Icon { name = "pen", size = 14 },
			},
			kx.Button {
				tone = "ghost",
				size = "sm",
				icon = true,
				label = "Remove",
				tooltip = "Remove from the team",
				on_click = function()
					store.delete_member(m.id)
					load()
				end,
				kx.Icon { name = "trash-bin-minimalistic", size = 14 },
			},
		},
	}
end

-- The teammate being added or changed, in a dialog.
local function editor(): Node
	local name = ui.state("team.e.name", "")
	local role = ui.state("team.e.role", "")
	local brief = ui.state("team.e.brief", "")
	local agent = ui.state("team.e.agent", "")
	ui.effect(function()
		local e = editing:get()
		if e then
			name:set(e.name)
			role:set(e.role)
			brief:set(e.instructions)
			agent:set(e.harness)
		end
	end)
	local open = ui.computed(function()
		return editing:get() ~= nil
	end)
	local function save()
		local e = editing:peek()
		if not e or string.match(name:peek(), "^%s*$") then
			return
		end
		store.save_member({
			id = e.id,
			name = name:peek(),
			role = role:peek(),
			avatar = e.avatar,
			hue = e.hue,
			harness = agent:peek(),
			instructions = brief:peek(),
		}):after(function()
			editing:set(nil)
			load()
		end, function(why: any)
			kx.toast { title = "That couldn't be saved", body = tostring(why), kind = "error" }
		end)
	end
	local options = ui.computed(function()
		local out = { { label = "Any agent", value = "" } }
		for _, a in agents:get() do
			if a.installed then
				table.insert(out, { label = a.label, value = a.id })
			end
		end
		return out
	end)
	return kx.Dialog {
		open = open,
		on_close = function()
			editing:set(nil)
		end,
		label = "Teammate",
		w = 460,
		ui.column {
			w_full = true,
			gap = "$space.3",
			ui.text {
				function()
					local e = editing:get()
					return if e and e.id ~= "" then "Edit teammate" else "Add a teammate"
				end,
				literal = true,
				text_size = "$font.size.lg",
				weight = "$font.weight.medium",
			},
			kx.Field { value = name, placeholder = "Name", label = "Name", w_full = true },
			kx.Field { value = role, placeholder = "Role (Reviewer, Frontend lead…)", label = "Role", w_full = true },
			ui.column {
				w_full = true,
				gap = "$space.1",
				kx.Hint { "Prefers", literal = true },
				ui.each(options, function(o: { label: string, value: string })
					return ui.button {
						h = 26,
						px = "$space.2",
						gap = "$space.1",
						items = "center",
						rounded = "$radius.md",
						cursor = "pointer",
						bg = function()
							return if agent:get() == o.value then "$selection.item" else "$kx.secondary.bg"
						end,
						border = 1,
						border_color = function()
							return if agent:get() == o.value then "$border.strong" else "#00000000"
						end,
						on_click = function()
							agent:set(o.value)
						end,
						if o.value ~= "" then kx.AgentMark { agent = o.value, size = 12 } else nil,
						ui.text { o.label, literal = true, text_size = "$font.size.sm" },
					}
				end, {
					key = function(o)
						return o.value
					end,
					direction = "row",
					flex_wrap = true,
					gap = "$space.1",
				}),
			},
			ui.textarea {
				value = brief,
				placeholder = "How they work: tone, standards, what to always or never do",
				placeholder_color = "$text.tertiary",
				label = "How they work",
				wrap = true,
				w_full = true,
				h = 120,
				p = "$space.3",
				rounded = "$radius.lg",
				bg = "$surface.base",
				border = 1,
				border_color = "$border.subtle",
				focus = { border_color = "$focus.ring" },
				text_size = "$font.size.md",
			},
			ui.row {
				w_full = true,
				gap = "$space.2",
				justify = "end",
				kx.Button { tone = "ghost", on_click = function()
					editing:set(nil)
				end, "Cancel" },
				kx.Button { on_click = save, "Save" },
			},
		},
	}
end

function M.view(): Node
	load()
	return ui.column {
		grow = 1,
		min_w = 0,
		h_full = true,
		ui.row { h = "$kx.titlebar", w_full = true, window_drag = true },
		ui.scroll {
			grow = 1,
			min_h = 0,
			w_full = true,
			ui.column {
				w_full = true,
				max_w = 960,
				px = "$space.7",
				pb = "$space.7",
				ui.text {
					"Team",
					literal = true,
					font = "$kx.display",
					text_size = "$kx.hero.size",
					line_height = "$kx.hero.line",
				},
				kx.Caption { "Your agents, and the teammates they work as.", literal = true },
				heading("Agents", "Every agent Keplorayx knows. Install one and it shows up here on its own."),
				ui.row {
					w_full = true,
					gap = "$space.2",
					flex_wrap = true,
					ui.each(agents, agent_card, {
						key = function(a: Agent)
							return a.id
						end,
						gap = "$space.2",
						flex_wrap = true,
						direction = "row",
					}),
				},
				ui.row {
					w_full = true,
					items = "end",
					heading("Teammates", "A name, a role and a way of working. Start a thread as one, and its brief leads the first turn."),
					kx.Button {
						tone = "secondary",
						size = "sm",
						mb = "$space.2",
						on_click = function()
							editing:set({ id = "", name = "", role = "", avatar = "", hue = 0.6, harness = "", instructions = "" })
						end,
						kx.Icon { name = "plus", size = 14 },
						"Add",
					},
				},
				ui.show(
					function()
						return #members:get() > 0
					end,
					ui.row {
						w_full = true,
						flex_wrap = true,
						gap = "$space.3",
						ui.each(members, member_card, {
							key = function(m: Member)
								return m.id
							end,
							gap = "$space.3",
							flex_wrap = true,
							direction = "row",
						}),
					},
					kx.Empty {
						icon = "teammate",
						title = "No teammates yet",
						hint = "Add a reviewer, a frontend lead, a writer: each one a brief your agents work by.",
					}
				),
			},
		},
		editor(),
	}
end

return M
