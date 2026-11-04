--!strict
-- Pull requests: the project's open ones on GitHub, through GitHub's own
-- command line (`gh`, signed in as you). Each shows its state, branch,
-- size and review, and opens on GitHub. Without `gh`, it says so and how
-- to get it.

local kx = require("@kx")
local store = require("@kx/store")

local M = {}

type Pull = {
	number: number,
	title: string,
	author: string,
	branch: string,
	state: string,
	draft: boolean,
	url: string,
	updated: string,
	review: string,
	added: number,
	removed: number,
}

local pulls = ui.state("pulls.list", {} :: { Pull })
local failure = ui.state("pulls.failure", "")
local loading = ui.state("pulls.loading", false)

local function load()
	loading:set(true)
	store.pulls(nil):after(function(list: any)
		pulls:set(list or {})
		failure:set("")
		loading:set(false)
	end, function(why: any)
		failure:set(tostring(if type(why) == "table" then why.message else why))
		loading:set(false)
	end)
end

local function state_of(p: Pull): (string, string)
	if p.draft then
		return "Draft", "$text.tertiary"
	elseif string.upper(p.state) == "MERGED" then
		return "Merged", "$kx.status.input"
	elseif string.upper(p.state) == "CLOSED" then
		return "Closed", "$danger"
	end
	return "Open", "$success"
end

local REVIEW = {
	APPROVED = "Approved",
	CHANGES_REQUESTED = "Changes asked for",
	REVIEW_REQUIRED = "Review needed",
}

local function row(p: Pull): Node
	local word, colour = state_of(p)
	return ui.button {
		w_full = true,
		p = "$space.2",
		gap = "$space.2",
		items = "start",
		rounded = "$radius.lg",
		cursor = "pointer",
		hover = { bg = "$hover" },
		label = p.title,
		on_click = function()
			pcall(ui.invoke, "open_url", p.url)
		end,
		ui.column { color = colour, pt = "$kx.space.hair", kx.Icon { name = if p.draft then "pull-request-draft" else "pull-request", size = 16 } },
		ui.column {
			grow = 1,
			min_w = 0,
			gap = "$kx.space.hair",
			ui.text { p.title, literal = true, truncate = true, w_full = true, text_size = "$font.size.md", weight = "$font.weight.medium" },
			ui.row {
				gap = "$space.2",
				items = "center",
				flex_wrap = true,
				ui.text { `#{p.number}`, literal = true, text_size = "$font.size.sm", color = "$text.tertiary", tabular = true },
				ui.text { word, literal = true, text_size = "$font.size.sm", color = colour },
				ui.row {
					gap = "$kx.space.hair",
					items = "center",
					color = "$text.tertiary",
					kx.Icon { name = "git-branch", size = 11 },
					ui.text { p.branch, literal = true, text_size = "$font.size.sm", truncate = true, max_w = 160 },
				},
				ui.text { `+{p.added}`, literal = true, text_size = "$font.size.sm", color = "$kx.syntax.inserted", tabular = true },
				ui.text { `−{p.removed}`, literal = true, text_size = "$font.size.sm", color = "$kx.syntax.deleted", tabular = true },
				if REVIEW[p.review]
					then ui.text { REVIEW[p.review], literal = true, text_size = "$font.size.sm", color = "$text.secondary" }
					else nil,
			},
			ui.text { `{p.author} · {p.updated}`, literal = true, text_size = "$font.size.xs", color = "$text.tertiary" },
		},
	}
end

function M.view(): Node
	load()
	return ui.column {
		grow = 1,
		min_h = 0,
		w_full = true,
		ui.row {
			w_full = true,
			px = "$space.3",
			pb = "$space.2",
			items = "center",
			kx.Hint { "Open on GitHub, through gh", literal = true, grow = 1 },
			kx.Button {
				tone = "ghost",
				size = "sm",
				icon = true,
				label = "Read them again",
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
				icon = "pull-request",
				title = "Pull requests can't be read",
				hint = "Install GitHub's command line (winget install GitHub.cli), then run `gh auth login` once.",
			},
			ui.show(
				function()
					return #pulls:get() == 0 and not loading:get()
				end,
				kx.Empty { icon = "pull-request", title = "No pull requests", hint = "None are open on this project's GitHub." },
				ui.scroll {
					grow = 1,
					min_h = 0,
					w_full = true,
					px = "$space.1",
					ui.each(pulls, row, {
						key = function(p: Pull)
							return tostring(p.number)
						end,
						w_full = true,
						gap = "$kx.space.hair",
					}),
				}
			)
		),
	}
end

return M
