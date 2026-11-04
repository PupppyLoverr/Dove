--!strict
-- A thread mid-turn: a prompt, thinking, tools with file changes, a reply
-- and the next one streaming.
local common = require("@fixtures/common")

local reply = [[The notch now lives in its own overlay window: always on top, transparent, and click-through until you hover it. Three things changed:

1. **`island.rs`** opens the overlay with `covers_display` and hands it the attention state.
2. **`notch.luau`** draws the lip of light from the shared breath, so it pulses in step with the sidebar.
3. Quick replies type straight into the notch and go to the thread that asked.

Run `cargo run -p keplorayx -- island` to try it.]]

return {
	projects = common.projects,
	agents = common.agents,
	threads = common.threads,
	thread = { id = "t1", title = "Port the notch to a zgx overlay", harness = "Claude Code", running = true, status = "Writing notch.luau" },
	entries = {
		{ id = "u1", kind = "user", text = "Port the notch to a zgx overlay window. Keep the lip of light and make quick replies work." },
		{ id = "k1", kind = "thinking", text = "The overlay needs covers_display and pointer passthrough until hover; the attention state is already shared…" },
		{
			id = "t1",
			kind = "tool",
			text = "Read apps/dove/src/notch/mod.rs",
			tool = { name = "Read", state = "done", output = "", changes = {} },
		},
		{
			id = "t2",
			kind = "tool",
			text = "Edit apps/keplorayx/src/island.rs",
			tool = {
				name = "Edit",
				state = "done",
				output = "",
				changes = { { path = "apps/keplorayx/src/island.rs", added = 84, removed = 6 } },
			},
		},
		{
			id = "t3",
			kind = "tool",
			text = "Run cargo check -p keplorayx",
			tool = { name = "Bash", state = "done", output = "Finished dev profile in 12.4s", changes = {} },
		},
		{ id = "a1", kind = "assistant", text = reply },
		{ id = "u2", kind = "user", text = "Now make the notch's quick reply keep focus after sending." },
		{
			id = "t4",
			kind = "tool",
			text = "Edit apps/keplorayx/ui/notch.luau",
			tool = {
				name = "Edit",
				state = "running",
				output = "",
				changes = { { path = "apps/keplorayx/ui/notch.luau", added = 12, removed = 3 } },
			},
		},
	},
	changes = {
		{
			path = "apps/keplorayx/src/island.rs",
			status = "added",
			binary = false,
			added = 84,
			removed = 0,
			hunks = {
				{
					header = "@@ -0,0 +1,12 @@",
					lines = {
						{ kind = "add", new = 1, text = "//! The notch on zgx: an overlay window over the display's top edge," },
						{ kind = "add", new = 2, text = "//! always on top, click-through until you hover it." },
						{ kind = "add", new = 3, text = "" },
						{ kind = "add", new = 4, text = "pub fn open(cx: &mut WindowContext, attention: Shared) {" },
						{ kind = "add", new = 5, text = "    let window = cx.windows.open(overlay(), move |cx| notch(cx, attention));" },
						{ kind = "add", new = 6, text = "    window.set_pointer_passthrough(true);" },
						{ kind = "add", new = 7, text = "}" },
					},
				},
			},
		},
		{
			path = "apps/keplorayx/ui/notch.luau",
			status = "modified",
			binary = false,
			added = 12,
			removed = 3,
			hunks = {
				{
					header = "@@ -40,9 +40,18 @@ local function reply()",
					lines = {
						{ kind = "ctx", old = 40, new = 40, text = "local function send()" },
						{ kind = "del", old = 41, text = "\tfield:blur()" },
						{ kind = "add", new = 41, text = "\t-- Keep the caret: the next reply follows at once." },
						{ kind = "add", new = 42, text = "\tfield:focus()" },
						{ kind = "ctx", old = 42, new = 43, text = "\ttext:set(\"\")" },
					},
				},
			},
		},
	},
	draft = { id = "a2", kind = "assistant", text = "Focus now stays in the reply field: after a send, the field clears and keeps the caret, so" },
}
