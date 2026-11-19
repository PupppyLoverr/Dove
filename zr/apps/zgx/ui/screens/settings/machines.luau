-- Settings → Machines: computers you already own, reached over your own
-- ssh (src/kx/remote.rs, docs/cloud-byok.md). A machine added here runs the
-- app's threads on its own engine: its data, its keys, its project — and the
-- sidebar says so while one is attached.
--
-- Nothing of this page exists in a working app until a machine is added:
-- local-only is the whole product, and there is no "cloud off" state to
-- miss.

local ds = require("@ds")
local toast = require("@ds/toast")
local search = require("@app/screens/settings/search")
local machines = require("@app/kx/machines")

local M = {}

M.ID = "machines"
M.TITLE = "Machines"

search.index(M.ID, ui.t("settings.machines.title"), ui.t("settings.machines.terms"), {
	{ key = "machines", title = ui.t("settings.machines.yours.label"), terms = ui.t("settings.machines.yours.terms") },
	{ key = "machines", title = ui.t("settings.machines.add.label"), terms = ui.t("settings.machines.add.terms") },
})

-- The dialog being filled in, and what it is waiting on.
local adding = ui.state("kx_machines_adding", false)
local target = ui.state("kx_machines_target", "")
local port = ui.state("kx_machines_port", "22")
local identity = ui.state("kx_machines_identity", "")
local busy = ui.state("kx_machines_busy", false)
-- The host key a machine offered, waiting to be accepted.
local offered = ui.state("kx_machines_offered", nil :: any)
-- The machine about to be forgotten.
local leaving = ui.state("kx_machines_leaving", nil :: any)

local function why(error: any): string
	if type(error) == "string" then
		return error
	end
	if type(error) == "table" and type(error.message) == "string" then
		return error.message
	end
	return tostring(error)
end

local function failed(title: string, error: any)
	toast.show { title = title, body = why(error), tone = "danger" }
end

local function add()
	local address = string.match(target:peek(), "^%s*(.-)%s*$") or ""
	if address == "" then
		return
	end
	local number = tonumber(port:peek()) or 22
	busy:set(true)
	ui.call("kx.remote.add", {
		target = address,
		port = number,
		identity = string.match(identity:peek(), "^%s*(.-)%s*$") or "",
	}):after(function(res: any)
		busy:set(false)
		adding:set(false)
		offered:set(res)
	end, function(error: any)
		busy:set(false)
		failed(ui.t("settings.machines.couldnt_add"):get(), error)
	end)
end

-- The answer to "accept this machine's host key?": yes pins it and connects,
-- no forgets the machine (nothing was kept but its name).
local function answer(accept: boolean)
	local machine = offered:peek()
	offered:set(nil)
	if not machine then
		return
	end
	ui.call("kx.remote.trust", { id = machine.id, accept = accept }):after(function()
		machines.refresh()
		if not accept then
			return
		end
		ui.call("kx.remote.connect", { id = machine.id }):after(function()
			machines.refresh()
		end, function(error: any)
			machines.refresh()
			failed(ui.t("settings.machines.couldnt_connect"):get(), error)
		end)
	end, function(error: any)
		failed(ui.t("settings.machines.couldnt_save"):get(), error)
	end)
end

local function connect(h: machines.Machine)
	ui.call("kx.remote.connect", { id = h.id }):after(function()
		machines.refresh()
	end, function(error: any)
		machines.refresh()
		failed(ui.t("settings.machines.couldnt_connect"):get(), error)
	end)
end

local function disconnect()
	ui.call("kx.remote.disconnect")
	machines.refresh()
end

local function forget()
	local h = leaving:peek()
	leaving:set(nil)
	if not h then
		return
	end
	ui.call("kx.remote.remove", { id = h.id })
	machines.refresh()
end

-- (Its words are a message, never a string read at build time: the label
-- follows the language as it changes.)
local function button(words: any, on_click: () -> (), tone: string?): Node
	return ds.Button {
		words,
		tone = tone or "secondary",
		size = "sm",
		on_click = function()
			on_click()
		end,
	}
end

-- What a machine's row offers, by what it is doing: connect it, accept its
-- key, leave it, or go back to this computer; and forget it either way.
local function controls(h: State<machines.Machine>): Node
	local machine = h:peek()
	local main = if machine.active
		then button(ui.t("settings.machines.disconnect"), disconnect)
		elseif not machine.trusted
			then button(ui.t("settings.machines.accept_key"), function()
				offered:set({ id = machine.id, fingerprint = machine.fingerprint, name = machine.name })
			end)
		else button(ui.t("settings.machines.connect"), function()
			connect(machine)
		end)
	return ui.row {
		gap = "$space.1",
		items = "center",
		main,
		button(ui.t("settings.machines.forget.action"), function()
			leaving:set(machine)
		end, "ghost"),
	}
end

function M.view(): Node
	machines.polling()
	local list = ui.computed(function(): { machines.Machine }
		return machines.now().hosts or {}
	end)
	local function here(): machines.Machine?
		for _, h in list:get() do
			if h.active then
				return h
			end
		end
		return nil
	end
	return ui.column {
		w_full = true,
		items = "center",
		ds.Settings.Page {
			title = ui.t("settings.machines.title"),
			description = ui.t("settings.machines.description"),
			search.anchor(
				"machines",
				ds.Settings.Section {
					title = ui.t("settings.machines.yours.label"),
					ui.show(function(): boolean
						return #list:get() == 0
					end, ds.EmptyState {
						icon = "remote-server",
						title = ui.t("settings.machines.empty_title"),
						body = ui.t("settings.machines.empty_body"),
						size = "sm",
					}),
					ds.Settings.Each(list, function(_h: machines.Machine, _i: number, h: State<machines.Machine>): Node
						local word, tone = machines.describe(h:peek())
						return ds.Settings.Row {
							label = function(): string
								return h:get().name
							end,
							description = function(): string
								return h:get().detail
							end,
							detail = function(): string
								return ui.t("settings.machines.reads", { dir = h:get().data_dir }):get()
							end,
							icon = if h:peek().here then "laptop" else "remote-server",
							badge = ds.Badge { word, tone = tone, size = "sm" },
							truncate = true,
							control = controls(h),
						}
					end, {
						key = function(h: machines.Machine): string
							return h.id
						end,
					}),
				}
			),
			search.anchor(
				"add",
				ds.Settings.Section {
					title = ui.t("settings.machines.add.label"),
					ds.Settings.Row {
						label = ui.t("settings.machines.add.label"),
						description = ui.t("settings.machines.add.description"),
						icon = "plus",
						control = button(ui.t("settings.machines.add.action"), function()
							adding:set(true)
						end, "primary"),
					},
					ui.show(function(): boolean
						return here() ~= nil
					end, ds.Settings.Row {
						label = ui.t("settings.machines.home.label"),
						description = ui.t("settings.machines.home.description"),
						icon = "laptop",
						control = button(ui.t("settings.machines.home.action"), disconnect),
					}),
				}
			),
		},
		ds.Dialog {
			open = adding,
			title = ui.t("settings.machines.add.title"),
			body = ui.t("settings.machines.add.body"),
			size = "md",
			confirm = {
				label = ui.t("settings.machines.add.action"),
				keep_open = true,
				busy = function(): boolean
					return busy:get()
				end,
				on_click = add,
			},
			ds.TextField {
				value = target,
				name = ui.t("settings.machines.add.target"),
				placeholder = "you@your-server",
				w_full = true,
				autofocus = true,
				on_commit = add,
			},
			ds.TextField {
				value = port,
				name = ui.t("settings.machines.add.port"),
				placeholder = "22",
				w_full = true,
			},
			ds.TextField {
				value = identity,
				name = ui.t("settings.machines.add.key"),
				placeholder = "~/.ssh/id_ed25519",
				w_full = true,
			},
		},
		ds.Dialog {
			open = ui.computed(function(): boolean
				return offered:get() ~= nil
			end),
			title = ui.t("settings.machines.key.title"),
			body = function(): string
				local machine = offered:get()
				if not machine then
					return ""
				end
				return ui.t("settings.machines.key.body", { name = machine.name, fingerprint = machine.fingerprint }):get()
			end,
			size = "md",
			confirm = {
				label = ui.t("settings.machines.key.accept"),
				on_click = function()
					answer(true)
				end,
			},
			cancel = ui.t("settings.machines.key.cancel"),
			ui.text {
				function(): string
					local machine = offered:get()
					return machine and machine.fingerprint or ""
				end,
				literal = true,
				font = "$font.mono",
				text_size = "$font.size.sm",
				color = "$text.secondary",
				wrap = true,
			},
		},
		ds.Dialog {
			open = ui.computed(function(): boolean
				return leaving:get() ~= nil
			end),
			title = ui.t("settings.machines.forget.title"),
			body = function(): string
				local machine = leaving:get()
				if not machine then
					return ""
				end
				return ui.t("settings.machines.forget.body", { name = machine.name }):get()
			end,
			tone = "danger",
			size = "sm",
			confirm = {
				label = ui.t("settings.machines.forget.action"),
				on_click = forget,
			},
		},
	}
end

return M
