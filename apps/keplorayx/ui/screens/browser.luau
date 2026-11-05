--!strict
-- The browser in the panel: a real web page (WebView2 on Windows, WebKit on
-- macOS and Linux) under a toolbar of Back, Forward, Reload and the
-- address, as Zeron's panel browser is (screens/panel/browser; MIT, see
-- THIRD_PARTY_NOTICES.md). Local dev servers open as http; words search.
-- One page a project, kept by the host with its history across reloads.

local kx = require("@kx")
local store = require("@kx/store")
local api = require("@screens/browser_api")

local M = {}

local ENGINE = "https://duckduckgo.com/?q=%s"
local typed = ui.state("browser.typed", "")
local editing = ui.state("browser.editing", false)
local refusal = ui.state("browser.refusal", "")
-- Where each page last was, by its id, so a new launch opens it again.
local last = ui.persist("browser.last", {} :: { [string]: string })

local function page_id(): string
	local t = store.thread:get()
	if not t then
		return "home"
	end
	for _, th in store.threads:get() do
		if th.id == t.id then
			return th.project_id
		end
	end
	return "home"
end

local QUICK = {
	{ label = "localhost:3000", url = "http://localhost:3000/" },
	{ label = "localhost:5173", url = "http://localhost:5173/" },
	{ label = "localhost:8080", url = "http://localhost:8080/" },
}

-- Opens `url` in the browser tab (Settings › General › Open links in).
function M.open_url(url: string)
	local id = page_id()
	api.web(`kx-browser-{id}`):load(url)
	local next = table.clone(last:peek())
	next[id] = url
	last:set(next)
end

local function page(id: string): Node
	local web = api.web(`kx-browser-{id}`)
	local function go(text: string)
		local url, why = api.resolve(text, ENGINE)
		if not url then
			refusal:set(why or "That isn't an address")
			return
		end
		refusal:set("")
		web:load(url)
		local next = table.clone(last:peek())
		next[id] = url
		last:set(next)
		editing:set(false)
	end
	local started = last:peek()[id]
	if started and (web.url == nil or web.url == "") then
		web:load(started)
	end
	local function blank(): boolean
		local u = web.url
		return u == nil or u == "" or u == "about:blank"
	end
	local function tool(icon: string, label: string, run: () -> (), enabled: (() -> boolean)?): Node
		return kx.Button {
			tone = "ghost",
			size = "xs",
			icon = true,
			label = label,
			tooltip = label,
			disabled = if enabled
				then function()
					return not enabled()
				end
				else nil,
			on_click = run,
			kx.Icon { name = icon, size = 14 },
		}
	end
	local toolbar = ui.row {
		w_full = true,
		h = 36,
		px = "$space.2",
		gap = "$space.1",
		items = "center",
		tool("arrow-left", "Back", function()
			web:back()
		end, function()
			return web.can_go_back == true
		end),
		tool("arrow-right", "Forward", function()
			web:forward()
		end, function()
			return web.can_go_forward == true
		end),
		ui.switch(function(): string
			return if web.loading then "stop" else "reload"
		end, {
			stop = tool("close", "Stop", function()
				web:stop()
			end),
			reload = tool("refresh", "Reload", function()
				web:reload()
			end),
		}),
		ui.input {
			value = typed,
			placeholder = "Search or type an address",
			placeholder_color = "$text.tertiary",
			label = "Address",
			grow = 1,
			min_w = 0,
			h = 26,
			px = "$space.2",
			rounded = "$radius.md",
			bg = "$surface.base",
			border = 1,
			border_color = "$border.subtle",
			focus = { border_color = "$focus.ring" },
			text_size = "$font.size.sm",
			on_focus = function()
				editing:set(true)
				typed:set(web.url or "")
			end,
			on_blur = function()
				editing:set(false)
			end,
			on_key = function(e: any): boolean?
				if e.key == "Enter" then
					go(typed:peek())
					return true
				elseif e.key == "Escape" then
					editing:set(false)
					return true
				end
				return nil
			end,
		},
		tool("arrow-up-right", "Open in your browser", function()
			if web.url then
				api.open_externally(web.url)
			end
		end, function()
			return not blank()
		end),
	}
	-- What the address shows when you aren't typing in it.
	ui.effect(function()
		if not editing:get() then
			local u = web.url
			typed:set(if u and u ~= "about:blank" then u else "")
		end
	end)
	local progress = ui.div {
		h = 2,
		w = function(): any
			local p = web.progress or 0
			return if web.loading then `{math.floor(math.clamp(p, 0.05, 1) * 100)}%` else "0%"
		end,
		bg = "$accent",
		opacity = function(): number
			return if web.loading then 1 else 0
		end,
	}
	local start = ui.column {
		grow = 1,
		w_full = true,
		items = "center",
		justify = "center",
		gap = "$space.3",
		p = "$space.5",
		ui.column { color = "$text.tertiary", kx.Icon { name = "global", size = 28 } },
		kx.Label { "Open a page", literal = true },
		kx.Caption {
			"Your dev server, docs, anything. Words search the web.",
			literal = true,
			align = "center",
			wrap = true,
			max_w = 300,
		},
		ui.row {
			gap = "$space.1",
			flex_wrap = true,
			justify = "center",
			table.unpack((function()
				local out: { Node } = {}
				for _, q in QUICK do
					table.insert(out, kx.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							go(q.url)
						end,
						q.label,
					})
				end
				return out
			end)()),
		},
	}
	return ui.column {
		grow = 1,
		min_h = 0,
		w_full = true,
		toolbar,
		ui.row { w_full = true, h = 2, progress },
		ui.show(function()
			return refusal:get() ~= ""
		end, kx.Hint {
			function()
				return refusal:get()
			end,
			literal = true,
			px = "$space.3",
			pb = "$space.1",
			color = "$danger",
		}),
		ui.column {
			grow = 1,
			min_h = 0,
			w_full = true,
			relative = true,
			api.view {
				id = `kx-browser-view-{id}`,
				label = "Web page",
				web = web,
				w_full = true,
				grow = 1,
				min_h = 0,
			},
			ui.show(blank, ui.column {
				absolute = true,
				top = 0,
				left = 0,
				right = 0,
				bottom = 0,
				bg = function()
					return if kx.material.see_through() then "$surface.base" else "$kx.solid"
				end,
				start,
			}),
		},
	}
end

function M.view(): Node
	if store.fixture ~= "" then
		return kx.Empty {
			icon = "global",
			title = "The browser",
			hint = "A web page opens here: your dev server, docs, anything.",
		}
	end
	return ui.presence(page_id, page, { grow = 1, min_h = 0, w_full = true })
end

return M
