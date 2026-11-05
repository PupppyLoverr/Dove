--!strict
-- The engine as the screens see it (src/host.rs): shared state and calls,
-- in one place, so a fixture can stand in for the host. With a fixture
-- asked for (`KX_FIXTURE=<name>`, ui/fixtures/<name>.luau), the states are
-- the fixture's and the calls are its stubs: every screen runs headless.

export type Thread = {
	id: string,
	title: string,
	project: string,
	project_id: string,
	status: string,
	harness: string,
	harness_id: string,
	updated: number,
	unread: boolean,
	-- Where it works and with what (the host gives these; a fixture may not).
	branch: string?,
	model: string?,
	pinned: boolean?,
	worktree: boolean?,
}

export type Project = { id: string, name: string, path: string }

export type Change = { path: string, added: number, removed: number }

export type Tool = { name: string, state: string, output: string, changes: { Change } }

export type Entry = { id: string, kind: string, text: string, tool: Tool? }

export type Draft = { id: string, kind: string, text: string }

export type Open = { id: string, title: string, harness: string, running: boolean, status: string }

export type Question = { text: string, options: { string } }

export type Approval = { request_id: string, tool: string, summary: string, questions: { Question } }

export type Agent = { id: string, label: string }

local fixture_name = (ui.shared("dev.fixture", "") :: any):peek() :: string

local store = {}

store.fixture = fixture_name

if fixture_name ~= "" then
	local ok, data = pcall(require, "@fixtures/" .. fixture_name)
	local f: any = if ok then data else {}
	store.threads = ui.state("fx.threads", (f.threads or {}) :: { Thread })
	store.projects = ui.state("fx.projects", (f.projects or {}) :: { Project })
	store.thread = ui.state("fx.thread", f.thread :: Open?)
	store.entries = ui.state("fx.entries", (f.entries or {}) :: { Entry })
	store.draft = ui.state("fx.draft", (f.draft or { id = "", kind = "", text = "" }) :: Draft)
	store.approval = ui.state("fx.approval", f.approval :: Approval?)
	store.agents = ui.state("fx.agents", (f.agents or {}) :: { Agent })
	store.route = f.route
	-- The fixture as written, for a screen's own state (a panel open).
	store.fixture_data = f
else
	store.threads = ui.shared("kx.threads", {} :: { Thread })
	store.projects = ui.shared("kx.projects", {} :: { Project })
	store.thread = ui.shared("kx.thread", nil :: Open?)
	store.entries = ui.shared("kx.entries", {} :: { Entry })
	store.draft = ui.shared("kx.draft", { id = "", kind = "", text = "" } :: Draft)
	store.approval = ui.shared("kx.approval", nil :: Approval?)
	store.agents = ui.shared("kx.agents", {} :: { Agent })
end

local function stubbed(): boolean
	return fixture_name ~= ""
end

function store.open(id: string)
	if stubbed() then
		for _, t in store.threads:peek() do
			if t.id == id then
				store.thread:set({ id = t.id, title = t.title, harness = t.harness, running = false, status = "" })
			end
		end
		return
	end
	ui.invoke("kx.open", id)
end

function store.home()
	if stubbed() then
		store.thread:set(nil)
		return
	end
	ui.invoke("kx.new")
end

function store.interrupt()
	if not stubbed() then
		ui.invoke("kx.interrupt")
	end
end

function store.decide(request_id: string, decision: string, answers: { { string } }?)
	if stubbed() then
		store.approval:set(nil)
		return
	end
	ui.invoke("kx.decide", { request_id = request_id, decision = decision, answers = answers })
end

-- What a new thread uses: `project`, `harness` (the agent), `model`
-- ("<agent>:<model>"), `effort`, `access` ("supervised", "edits", "full" or
-- "plan": how much it may do before it asks) and `persona` (a teammate).
-- In a thread, the message goes to it as it is.
function store.send(text: string, with: { [string]: any }?): any
	if stubbed() then
		return ui.sleep(0)
	end
	local req = table.clone(with or {})
	req.text = text
	return ui.call("kx.send", req)
end

-- A call the host answers, or the fixture's own `stub[name](arg)` (or
-- `fallback`) when a fixture stands in.
local function call(name: string, arg: any, fallback: any): any
	if stubbed() then
		local ok, data = pcall(require, "@fixtures/" .. fixture_name)
		local stubs = if ok and type(data) == "table" then (data :: any).stubs else nil
		if not stubs then
			local has, common = pcall(require, "@fixtures/common")
			stubs = if has then (common :: any).stubs else nil
		end
		local f = stubs and stubs[name]
		local value = if f then f(arg) else fallback
		-- Settled already: `after` answers at once, `await` returns it.
		return {
			after = function(self: any, done: (any) -> (), _failed: any)
				done(value)
				return self
			end,
			await = function(_self: any)
				return value
			end,
		}
	end
	return ui.call(name, arg)
end
store.call = call

-- A project's folder, one level: { name, path, dir, size } sorted folders
-- first. `path` is relative to the project's root ("" for the root).
function store.list_dir(project: string?, path: string): any
	return call("kx.list_dir", { project = project, path = path }, {})
end

-- The project's every file at once: { name, root, files (one path a line),
-- statuses = { [path] = "modified" | "added" | … } }.
function store.list_files(project: string?): any
	return call("kx.list_files", { project = project }, { name = "", root = "", files = "", statuses = {} })
end

-- A file's text (capped), with its language by extension.
function store.read_file(project: string?, path: string): any
	return call("kx.read_file", { project = project, path = path }, { text = "", lang = "" })
end

-- The project's history: the last commits, newest first, each with its
-- parents (for the graph), its refs, author, when and subject.
function store.history(project: string?): any
	return call("kx.history", { project = project }, {})
end

-- The pull requests for the project's GitHub remote (gh, when it's there).
function store.pulls(project: string?): any
	return call("kx.pulls", { project = project }, {})
end

-- The app's settings, as Dove keeps them (its settings.json), read and
-- written by key.
function store.settings(): any
	return call("kx.settings", nil, {})
end

function store.set_setting(key: string, value: any): any
	return call("kx.set_setting", { key = key, value = value }, true)
end

-- The team: the agents installed on this device, with whether each is
-- signed in, and the orchestra's lanes.
function store.team(): any
	return call("kx.team", nil, { agents = {}, members = {} })
end

function store.save_member(member: { [string]: any }): any
	return call("kx.save_member", member, "fx")
end

function store.delete_member(id: string)
	if not stubbed() then
		ui.invoke("kx.delete_member", id)
	end
end

function store.add_project(path: string): any
	if stubbed() then
		return ui.sleep(0)
	end
	return ui.call("kx.add_project", path)
end

-- The open thread's changes (screens/changes), to `done(files)` or
-- `failed(why)` once read.
function store.changes(done: (any) -> (), failed: (any) -> ())
	if stubbed() then
		local ok, data = pcall(require, "@fixtures/" .. fixture_name)
		local list = if ok and type(data) == "table" then (data :: any).changes else nil
		done(list or {})
		return
	end
	ui.call("kx.changes", nil):after(done, failed)
end

-- A thread is the open one.
function store.is_open(id: string): boolean
	local t = store.thread:get()
	return t ~= nil and t.id == id
end

return store
