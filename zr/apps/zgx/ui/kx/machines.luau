--!strict
-- Machines you own (src/kx/remote.rs): the state the app is in, read for
-- the marker under the sidebar's list and for Settings › Machines. With no
-- machine added there is nothing here to draw and nothing polling: local
-- only is the product, not a degraded mode.
--
-- The link lives on a thread of the app's, so its state is read now and then
-- (`kx.remote.status`) rather than pushed; a change is what rebuilds
-- anything, and an unchanged read reaches no node at all.

local ds = require("@ds")
local route = require("@app/screens/shell/route")

local M = {}

export type Machine = {
	id: string,
	name: string,
	target: string,
	address: string,
	port: number,
	identity: string?,
	data_dir: string,
	engine_port: number,
	fingerprint: string?,
	trusted: boolean,
	state: string,
	detail: string,
	version: string?,
	active: boolean,
	here: boolean,
}

export type Snapshot = {
	hosts: { Machine },
	active: string?,
	active_name: string?,
	link: { host: string, port: number, for_seconds: number }?,
}

-- What the host last said (its `kx.remote` shared value, and each poll).
local held = ui.shared("kx.remote", { hosts = {}, active = nil } :: any)
local live = ui.state("kx_machines_live", nil :: any)

-- What last built anything: a poll that changes none of it is dropped, so a
-- window with a machine attached is not rebuilt four times a second.
local stamp = ""

function M.now(): Snapshot
	local latest = live:get()
	if latest then
		return latest :: Snapshot
	end
	return (held:get() or { hosts = {}, active = nil }) :: Snapshot
end

function M.refresh()
	local ok, got = pcall(ui.call_sync, "kx.remote.status")
	if not ok or type(got) ~= "table" then
		return
	end
	local parts: { string } = { tostring(got.active), tostring(got.link and got.link.port) }
	for _, h in got.hosts or {} do
		table.insert(parts, `{h.id}={h.state}/{h.detail}/{h.trusted}`)
	end
	local now = table.concat(parts, ";")
	if now == stamp then
		return
	end
	stamp = now
	live:set(got)
end

-- Reads the state every 800 ms, from the node using it: armed once per
-- mount (an effect reads nothing of its own, so it runs once), and gone with
-- the node. A window with no machine never arms it.
function M.polling(): ()
	ui.effect(function()
		ui.every(800, M.refresh)
	end)
end

function M.any(): boolean
	return #(M.now().hosts or {}) > 0
end

-- A machine's state in words and a tone, for its row and the marker.
function M.describe(h: Machine): (string, string)
	local state = h.state or "stopped"
	if state == "live" then
		return "Connected", "success"
	elseif state == "connecting" then
		return "Connecting", "warning"
	elseif state == "starting" then
		return "Starting its engine", "warning"
	elseif state == "reconnecting" then
		return "Reconnecting", "warning"
	elseif state == "untrusted" then
		return "Host key not accepted", "neutral"
	elseif state == "stopped" then
		return if h.trusted then "Not connected" else "Host key not accepted", "neutral"
	elseif state == "unreachable" then
		return "Unreachable", "danger"
	elseif state == "refused_key" then
		return "Key refused", "danger"
	elseif state == "host_key_changed" then
		return "Host key changed", "danger"
	elseif state == "no_engine" then
		return "No engine there", "danger"
	elseif state == "engine_stopped" then
		return "Engine not running", "danger"
	elseif state == "no_ssh" then
		return "No ssh here", "danger"
	end
	return "Failed", "danger"
end

-- Under the sidebar's list: where this window's threads are running, while
-- they are running somewhere other than this computer.
function M.line(): Node?
	return ui.show(function(): boolean
		return M.any()
	end, ui.component(function(): Node
		M.polling()
		local function active(): Machine?
			for _, h in M.now().hosts or {} do
				if h.active then
					return h
				end
			end
			return nil
		end
		local function tone(): string
			local h = active()
			return if h then (select(2, M.describe(h))) else "neutral"
		end
		local function words(): string
			local h = active()
			if not h then
				return ui.t("shell.sidebar.machine.here"):get()
			end
			local state, _ = M.describe(h)
			if state == "Connected" then
				return ui.t("shell.sidebar.machine.running_on", { name = h.name }):get()
			end
			return ui.t("shell.sidebar.machine.other", { state = state, name = h.name }):get()
		end
		return ui.button {
			w_full = true,
			h = 30,
			px = "$space.2",
			rounded = "$radius.lg",
			items = "center",
			gap = "$space.2",
			hover = { bg = "$hover" },
			label = words(),
			on_click = function()
				route.settings("machines")
			end,
			ds.Dot { tone = tone() },
			ui.text {
				words(),
				literal = true,
				text_size = "$font.size.sm",
				color = "$text.secondary",
				truncate = true,
				grow = 1,
			},
		}
	end)({}))
end

return M
