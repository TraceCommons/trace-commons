import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useState } from "react";
import {
  BarGraph,
  Breadcrumb,
  ButtonPrimary,
  ButtonSecondary,
  Card,
  Checkbox,
  CheckRow,
  Chip,
  ConsentBlock,
  Expander,
  EyebrowCard,
  FolderButton,
  GlassButton,
  Input,
  KeyValueList,
  LegendCell,
  ListRow,
  MapArc,
  MapNode,
  Menu,
  MenuItem,
  MenuSeparator,
  Modal,
  NodeCard,
  Notice,
  Pane,
  Picker,
  PillIconButton,
  QuietCard,
  Radio,
  RadioGroup,
  RoundButton,
  SectionRule,
  SegmentedTabs,
  Select,
  Skeleton,
  StatusDot,
  StatusLabel,
  StepProgress,
  SubmitPill,
  Tag,
  TertiaryLink,
  TextArea,
  TextField,
  Toggle,
  ToolLogo,
  ToolTile,
  WatchSwitch,
  Window,
} from ".";

const meta = {
  title: "Design System/Glass",
  parameters: { layout: "fullscreen" },
} satisfies Meta;
export default meta;
type Story = StoryObj<typeof meta>;

function Section({
  number,
  title,
  note,
  children,
}: {
  number: string;
  title: string;
  note: string;
  children: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-3">
      <div className="flex items-baseline gap-2.5">
        <span className="tc-chip tc-chip--glass">{number}</span>
        <span className="tc-display">{title}</span>
      </div>
      <p className="m-0 max-w-[960px] tc-body tc-text-secondary">{note}</p>
      <div className="grid grid-cols-3 gap-7">{children}</div>
    </section>
  );
}

function Specimen({
  label,
  spec,
  children,
}: {
  label: string;
  spec: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex min-h-[72px] flex-wrap items-center gap-2.5">
        {children}
      </div>
      <div className="tc-eyebrow">{label}</div>
      <div className="tc-mono whitespace-pre-wrap tc-text-secondary">{spec}</div>
    </div>
  );
}

function Swatch({ token }: { token: string }) {
  return (
    <span className="flex flex-col items-center gap-1">
      <span
        className="h-9 w-13 rounded-[10px]"
        style={{ background: `var(${token})` }}
      />
      <span className="tc-mono text-[10px] tc-text-tertiary">{token}</span>
    </span>
  );
}

function ControlsDemo() {
  const [toggle, setToggle] = useState(true);
  const [settingsToggle, setSettingsToggle] = useState(false);
  const [watch, setWatch] = useState(true);
  const [checked, setChecked] = useState(true);
  const [children, setChildren] = useState([true, false]);
  const [rule, setRule] = useState<"watch" | "ask" | "never" | null>("watch");
  const [tab, setTab] = useState<"home" | "inference" | "traces">("home");
  const [open, setOpen] = useState(true);
  const all = children.every(Boolean);
  const some = children.some(Boolean);
  return (
    <>
      <Specimen
        label="Primary · disabled · secondary"
        spec={"--tc-cta-fill · h34 · pad 0 16 · 13/700 white\ndisabled: opacity .45\n--tc-cta-secondary-fill (dark purple)"}
      >
        <ButtonPrimary>Start watching</ButtonPrimary>
        <ButtonPrimary disabled>Continue</ButtonPrimary>
        <ButtonSecondary>Download</ButtonSecondary>
      </Specimen>
      <Specimen
        label="Glass · submit pill · round · folder · tertiary"
        spec={"--tc-control-fill + --tc-control-edge · h28 (24 in rows)\nround 28×28 · folder: icon + “…”\ntertiary: --tc-purple-text"}
      >
        <GlassButton>Check Now</GlassButton>
        <SubmitPill>Submit · 3</SubmitPill>
        <SubmitPill done>Sent</SubmitPill>
        <RoundButton label="Settings">⚙</RoundButton>
        <FolderButton label="Choose a different folder…" />
        <PillIconButton label="Previous period">‹</PillIconButton>
        <TertiaryLink>Customize</TertiaryLink>
      </Specimen>
      <Specimen
        label="Picker (native select)"
        spec={"pill h28 · pad 0 8 0 10 · gap 6\ndot 7px · label 12/600 · chevron 10px\nunanswered: no dot, “Choose…”"}
      >
        <Picker
          label="Folder rule"
          value={rule}
          onChange={setRule}
          options={[
            { value: "watch", label: "Watch this folder", dot: "var(--tc-status-on)" },
            { value: "ask", label: "Ask me", dot: "var(--tc-status-ask)" },
            { value: "never", label: "Never", dot: "var(--tc-status-off)" },
          ]}
        />
        <Picker label="Unanswered" value={null} onChange={() => undefined} options={[{ value: "a", label: "A" }]} />
      </Specimen>
      <Specimen
        label="Toggle · watch switch · checkbox (group)"
        spec={"toggle 40×24 · on --tc-toggle-on / settings purple\nwatch switch 38×22 · on --tc-status-on\ncheckbox 15 · r5 · group toggles children"}
      >
        <Toggle label="Toggle" checked={toggle} onChange={setToggle} />
        <Toggle label="Settings toggle" settings checked={settingsToggle} onChange={setSettingsToggle} />
        <WatchSwitch label="Watch this folder" checked={watch} onChange={setWatch} />
        <Checkbox label="Single" checked={checked} onChange={setChecked} />
        <Checkbox
          label="Group"
          checked={all}
          indeterminate={some && !all}
          onChange={(next) => setChildren(children.map(() => next))}
        />
        {children.map((value, index) => (
          <Checkbox
            key={index === 0 ? "first" : "second"}
            label={`Child ${index + 1}`}
            checked={value}
            onChange={(next) =>
              setChildren(children.map((item, i) => (i === index ? next : item)))
            }
          />
        ))}
      </Specimen>
      <Specimen
        label="Segmented tabs (Home · Inference · Traces)"
        spec={"well · h30 · pad 2 · selected: control-selected + edge\nbadge: decisions owed, 10/700"}
      >
        <SegmentedTabs
          label="Monitor"
          className="w-[320px]"
          value={tab}
          onChange={setTab}
          items={[
            { value: "home", label: "Home" },
            { value: "inference", label: "Inference", dot: "var(--tc-status-on)" },
            { value: "traces", label: "Traces", badge: 9 },
          ]}
        />
      </Specimen>
      <Specimen
        label="Legend cells"
        spec={"one well per datum · h28 · dot 8 + 2px halo\nlabel secondary · count tabular 600"}
      >
        <div className="tc-legend w-[280px]">
          <LegendCell color="var(--tc-data-shared)" label="shared" value="218 KB" />
          <LegendCell color="var(--tc-data-kept)" label="kept" value="494 KB" />
        </div>
      </Specimen>
      <Specimen
        label="Step progress (FTUX)"
        spec={"nodes 14 · done/current purple-soft + 3px halo\nlines 2px · labels eyebrow upper"}
      >
        <div className="w-[380px]">
          <StepProgress labels={["Join", "Folders", "Uses"]} current={1} />
        </div>
      </Specimen>
      <Specimen
        label="Chip · tag · badge"
        spec={"chip: mono 11 · ring .5px colour@60%\ntag: tinted verdict · badge: decisions owed"}
      >
        <Chip tone="on">Scrubbed</Chip>
        <Chip tone="ask">Worth a second look</Chip>
        <Chip tone="outside">Outside · no proof</Chip>
        <Chip glass>always on</Chip>
        <Tag tone="on">Contributed</Tag>
        <Tag tone="ask">Under review</Tag>
        <Tag>Taken back</Tag>
        <span className="tc-badge">14</span>
      </Specimen>
      <Specimen
        label="Breadcrumb · expander · menu"
        spec={"back: round 26 · crumbs 12/600\nexpander: “›” rotates 90° · menu: popover tier"}
      >
        <Breadcrumb onBack={() => undefined} trail={[{ label: "Home", onSelect: () => undefined }, { label: "Missions" }]} />
        <Expander open={open} onToggle={() => setOpen(!open)}>
          Decisions
        </Expander>
        <Menu className="w-[220px]">
          <MenuItem checked onSelect={() => undefined}>
            Show ignored folders
          </MenuItem>
          <MenuSeparator />
          <MenuItem disabled>Show sessions of other users</MenuItem>
        </Menu>
      </Specimen>
      <FormDemo />
    </>
  );
}

function FormDemo() {
  const [path, setPath] = useState("ask_first");
  return (
    <>
      <Specimen
        label="Input · text area · select"
        spec={"field fill · r8 · focus 2px purple-text\nselect: glass pill + chevron, native menu"}
      >
        <div className="flex w-[280px] flex-col gap-2">
          <Input aria-label="Commons URL" placeholder="https://tracecommons.ai" />
          <TextArea aria-label="Outcome" placeholder="What happened" rows={2} />
          <Select aria-label="Outcome" defaultValue="worked">
            <option value="worked">Worked</option>
            <option value="partly">Partly</option>
            <option value="failed">Failed</option>
          </Select>
        </div>
      </Specimen>
      <Specimen
        label="Radio group"
        spec={"round checkbox · arrows move the choice\nthe wrapping label names each radio"}
      >
        <RadioGroup aria-label="Path" value={path} onChange={setPath} className="gap-1">
          <label className="flex items-start gap-2.5 tc-label font-normal">
            <Radio value="ask_first" />
            Ask me each time
          </label>
          <label className="flex items-start gap-2.5 tc-label font-normal">
            <Radio value="automatic" />
            Contribute automatically
          </label>
        </RadioGroup>
      </Specimen>
      <Specimen
        label="Notice · skeleton"
        spec={"quiet card · tone is the title's status dot\nno tinted fill, no coloured edge"}
      >
        <div className="flex w-[320px] flex-col gap-2">
          <Notice tone="ask" title="Automatic contributing stopped">
            orchard-api changed its settings. Nothing is sent until you look.
          </Notice>
          <Notice tone="outside" title="Rust core could not start">
            Source roots remain saved.
          </Notice>
          <Skeleton className="h-4 w-2/3" />
        </div>
      </Specimen>
    </>
  );
}

function ModalDemo() {
  const [open, setOpen] = useState(false);
  return (
    <div className="relative h-[260px] w-full overflow-hidden rounded-[16px]">
      <GlassButton onClick={() => setOpen(true)}>Open modal</GlassButton>
      <Modal open={open} onClose={() => setOpen(false)} title="Settings" subtitle="What this machine watches." narrow>
        <div className="p-4 tc-body">Modal tier: gradient rim, blur 30.</div>
      </Modal>
    </div>
  );
}

export const Library: Story = {
  render: () => (
    <Window className="min-h-screen flex-col gap-11 p-12">
      <Section
        number="01"
        title="Colour"
        note="Brand purple carries the one action and the shared series. Status colours live on dots and labels only. Text has three vibrancy tiers."
      >
        <Specimen label="Brand" spec={"--tc-purple · --tc-purple-soft\n--tc-purple-text · --tc-blue"}>
          <Swatch token="--tc-purple" />
          <Swatch token="--tc-purple-soft" />
          <Swatch token="--tc-purple-text" />
          <Swatch token="--tc-blue" />
        </Specimen>
        <Specimen label="Status · dot + label only" spec={"--tc-status-on · ask · off · outside"}>
          <StatusLabel tone="on">on · watching</StatusLabel>
          <StatusLabel tone="ask">ask me</StatusLabel>
          <StatusLabel tone="off">never</StatusLabel>
          <StatusLabel tone="outside">outside model</StatusLabel>
        </Specimen>
        <Specimen label="Text on glass · ≥4.5:1" spec={"--tc-text-primary · secondary · tertiary\n--tc-purple-text"}>
          <span className="flex flex-col gap-1">
            <span className="tc-text-primary">Primary · Nothing is sent unless you say so.</span>
            <span className="tc-text-secondary">Secondary · Scrubbed on this Mac.</span>
            <span className="tc-text-tertiary">Tertiary · Sat · 1 h 18 min · waiting</span>
            <span className="tc-text-accent">Accent · Customize</span>
          </span>
        </Specimen>
      </Section>
      <Section
        number="02"
        title="Materials"
        note="Four tiers. Panes blur the scene; cards tint the pane; wells recess into a card; controls float on top."
      >
        <Specimen label="Pane → card → well → control" spec={"--tc-pane-fill + blur 30 · r16\n--tc-card-fill · r14 · quiet r12\n--tc-well-fill · --tc-control-fill"}>
          <Pane padded className="flex w-full flex-col gap-2.5">
            <span className="tc-mono tc-text-tertiary">pane · blur 30 / saturate 180</span>
            <Card className="flex flex-col gap-2">
              <span className="tc-mono tc-text-tertiary">card · tint, no blur</span>
              <div className="flex items-center gap-2.5">
                <span className="tc-well h-[26px] flex-1" />
                <RoundButton label="Control">⚙</RoundButton>
              </div>
            </Card>
            <QuietCard>
              <span className="tc-mono tc-text-tertiary">quiet card · 12px</span>
            </QuietCard>
          </Pane>
        </Specimen>
        <Specimen label="Modal" spec={"--tc-modal-fill · --tc-rim-window\nscrim --tc-modal-scrim + blur 8"}>
          <ModalDemo />
        </Specimen>
        <Specimen label="Node card (map hover/pin)" spec={"popover tier · r14 · w260"}>
          <NodeCard title="NEAR AI credential" body="One key, held in the Keychain." hint="hover to peek · click to pin" />
        </Specimen>
      </Section>
      <Section
        number="03"
        title="Type & spacing"
        note="System SF Pro, SF Mono for paths and chips. 10 eyebrow, 11 caption/mono, 12 label, 13 body, 15 title, 17 heading, 22 display, 28 number."
      >
        <Specimen label="Scale" spec={"--tc-text-display · heading · title\nbody · label · eyebrow · mono"}>
          <span className="flex flex-col gap-1.5">
            <span className="tc-display">Which folders may this app watch?</span>
            <span className="tc-heading">orchard-api · add rate limiter</span>
            <span className="tc-title">Watching 3 tools</span>
            <span className="tc-body">Nothing is sent unless you say so.</span>
            <span className="tc-label">Share automatically</span>
            <span className="tc-eyebrow">Recent activity</span>
            <span className="tc-mono tc-text-tertiary">~/.claude/projects</span>
            <span className="tc-number">41.0</span>
          </span>
        </Specimen>
        <Specimen label="Radii" spec={"window 22 · pane 16 · card 14 · quiet 12 · control 8 · pill"}>
          {["--tc-radius-window", "--tc-radius-pane", "--tc-radius-card", "--tc-radius-card-sm", "--tc-radius-control", "--tc-radius-pill"].map((token) => (
            <span key={token} className="tc-card h-11 w-11" style={{ borderRadius: `var(${token})`, padding: 0 }} title={token} />
          ))}
        </Specimen>
        <Specimen label="Tool tiles" spec={"22 r6 · tinted inline svg 15"}>
          <ToolTile tool="claude" />
          <ToolTile tool="codex" />
          <ToolTile tool="anti" />
          <ToolTile tool="opencode" />
          <ToolTile tool="theia" />
          <ToolTile kind="folder" />
          <ToolTile kind="session" />
          <ToolLogo id="claude" size={24} />
        </Specimen>
      </Section>
      <Section number="04" title="Controls" note="Every interactive element, working.">
        <ControlsDemo />
      </Section>
      <Section
        number="05"
        title="Patterns"
        note="Tree row, card with eyebrow, consent block, section rule, key-value list, bar graph and map."
      >
        <Specimen label="Tree row · tool › folder › session" spec={"grid 16 24 1fr auto 38 22 · indent +18/level"}>
          <Pane className="flex w-full flex-col p-2" role="tree">
            <ListRow depth={0} logo="claude" title="Claude Code" sub="~/.claude/projects · 62 sessions" expanded onToggleExpand={() => undefined} submitLabel="Submit · 3" onSubmit={() => undefined} watched onToggleWatch={() => undefined} />
            <ListRow depth={1} title="orchard-api" sub="Ask me · 3 waiting" expanded onToggleExpand={() => undefined} submitLabel="Submit · 3" onSubmit={() => undefined} watched onToggleWatch={() => undefined} onOpenMenu={() => undefined} />
            <ListRow depth={2} title="add rate limiter" sub="Sat · 29 min · nothing matched" flag="ask" submitLabel="Review" onSubmit={() => undefined} onOpenMenu={() => undefined} selected />
          </Pane>
        </Specimen>
        <Specimen label="Card with eyebrow · consent block" spec={"card pad 12 14 · eyebrow 10/700\nconsent: well r12, verbatim, above Submit"}>
          <div className="flex w-full flex-col gap-2.5">
            <EyebrowCard eyebrow="Sharing" accessory={<StatusLabel tone="on">Share automatically</StatusLabel>}>
              <span className="tc-label font-normal tc-text-secondary">Scrubbed on this Mac, shared once clean.</span>
            </EyebrowCard>
            <ConsentBlock>
              "Exactly what would be sent" is the exact text that would leave this machine. Pattern-based scrubbing may have missed something in it, and nothing here checks that you looked.
            </ConsentBlock>
            <CheckRow checked onChange={() => undefined}>
              Notifications allowed
            </CheckRow>
          </div>
        </Specimen>
        <Specimen label="Section rule · key-value · field" spec={"settings eyebrow purple + rule\nkv 70px label column"}>
          <div className="flex w-full flex-col gap-2.5">
            <SectionRule>Watched folders</SectionRule>
            <KeyValueList items={[{ label: "Path", value: "~/code/orchard-api", mono: true }, { label: "Sessions", value: 18 }]} />
            <TextField label="Port" defaultValue="8463" />
          </div>
        </Specimen>
        <Specimen label="Bar graph (shared/kept)" spec={"shared rises purple · kept falls blue"}>
          <div className="w-full">
            <BarGraph
              scaleFloor={5}
              buckets={["Sat", "Sun", "Mon", "Tue", "Wed", "Thu", "Fri"].map((day, index) => ({
                key: day,
                label: day,
                up: [3, 1, 0, 4, 2, 0, 5][index],
                down: [2, 4, 1, 0, 3, 1, 2][index],
                description: day,
              }))}
            />
          </div>
        </Specimen>
        <Specimen label="Map node · arc" spec={"node r11 · ring when active · arc bends .28"}>
          <div className="tc-map relative h-[200px] w-full">
            <svg viewBox="0 0 300 200" className="absolute inset-0 h-full w-full" role="img" aria-label="Map sample">
              <MapArc from={[60, 140]} to={[240, 50]} tone="on" pulse />
              <MapArc from={[60, 140]} to={[240, 160]} tone="outside" />
              <MapNode x={60} y={140} r={16} fill="#8e8e96" ring label="This Mac" />
              <MapNode x={240} y={50} fill="var(--tc-blue)" ring label="Library" />
              <MapNode x={240} y={160} fill="var(--tc-status-outside)" label="Outside" />
            </svg>
          </div>
        </Specimen>
        <Specimen label="Status dots" spec={"7–8px · ring for the home status"}>
          <StatusDot tone="on" ring />
          <StatusDot tone="ask" ring />
          <StatusDot tone="shared" halo />
          <StatusDot tone="kept" halo />
          <StatusDot empty />
        </Specimen>
      </Section>
    </Window>
  ),
};
