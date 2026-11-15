--!strict
-- The chat composer docked at a page's foot, floating over the transcript,
-- and the room the transcript keeps under it: the chat page's (screens/chat
-- session and composer) and a side chat's tab (screens/chat/lane_tab), one
-- geometry, so the two read the same. The conversation runs behind the
-- composer, its end, the send's runway, the working row and the jump-to-
-- latest pill held clear of it by the transcript's `foot`.
--
--   Dock.GAP, Dock.FOOTER, Dock.REST   its gap to the page's foot; its
--                         footer's line (the chips under the pill: 24, and
--                         8 above them), which has no surface of its own;
--                         its height with one line of draft
--   Dock.page_pad()       the page's bottom padding: to the footer's line
--   Dock.room(height)     the transcript's `foot`, from the
--                         docked composer's measured height
--   Dock.Dock { height, composer, id? }   the composer at the page's foot,
--                         as wide as the conversation's column, its height
--                         measured into `height`; the page under it takes
--                         the pointer elsewhere

local prefs = require("@app/screens/settings/prefs")

local M = {}

M.GAP = 8
M.FOOTER = 32
M.REST = 81
-- Its sides' room from the page's edges.
M.SIDES = ui.raw(32, "the composer's room from the page's edges")

-- A chat's head: the height of a code pane's (`zeron.files.head`), so the two stand
-- level side by side.
function M.head(): number
	return (ui.theme.get("zeron.files.head") :: number?) or 40
end

function M.page_pad(): any
	return ui.raw(M.FOOTER + M.GAP, "the docked composer's footer line and its gap to the foot")
end

function M.room(height: () -> number): { foot: () -> number }
	return {
		-- (The viewport ends at the footer's line: what floats over it is the
		-- composer above that.)
		foot = function(): number
			return height() - M.FOOTER
		end,
	}
end

export type DockProps = { height: State<number>, composer: Node, id: string? }

-- Room taken at the page's right by something standing beside the chat (a
-- buddy's drawer, kx/buddies/chat_bar, sets this to a function of what
-- shows): the composer centres on what's left, so it never runs under it.
M.right_of = nil :: (() -> number)?
-- (Bumped once as `right_of` is set, at its module's load: a dock drawn
-- before then reads it again.)
M.right_ready = ui.state("chat_dock_right_ready", 0)

function M.Dock(props: DockProps): Node
	return ui.overlay {
		absolute = true,
		left = 0,
		right = 0,
		top = 0,
		bottom = 0,
		pointer_events = "none",
		ui.row {
			absolute = true,
			pointer_events = "none",
			left = 0,
			right = function(): number
				M.right_ready:get()
				local f = M.right_of
				return if f then f() else 0
			end,
			bottom = M.GAP,
			px = M.SIDES,
			justify = "center",
			ui.div {
				id = props.id,
				w_full = true,
				pointer_events = "auto",
				max_w = function(): number
					return prefs.conversation_width:get()
				end,
				on_resize = function(size: { w: number, h: number })
					if math.abs(props.height:peek() - size.h) > 0.01 then
						props.height:set(size.h)
					end
				end,
				props.composer,
			},
		},
	}
end

return M
