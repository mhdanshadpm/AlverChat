import { useMemo, useState } from 'react';
import {
  Avatar,
  Badge,
  BadgeCount,
  Banner,
  Button,
  ContextMenu,
  ContextMenuTriggerRegion,
  Dialog,
  EmptyState,
  Icon,
  IconButton,
  Input,
  ListGroup,
  ListRow,
  MessageBubble,
  ProgressBar,
  SearchInput,
  Segmented,
  Select,
  Skeleton,
  Slider,
  Tabs,
  ToastViewport,
  Toggle,
  Tooltip,
  Waveform,
  contextMenuItems,
  AVATAR_PALETTE,
  ICON_NAMES,
  PRESENCE_STATES,
  type ContextMenuEntry,
  type DeliveryState,
  type ToastView,
} from '../components';
import './gallery.css';

/** Tiny inline SVG posters so the gallery needs no network access at all. */
const poster = (from: string, to: string, label: string): string =>
  `data:image/svg+xml,${encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="360" height="240">' +
      '<defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">' +
      `<stop offset="0" stop-color="${from}"/><stop offset="1" stop-color="${to}"/></linearGradient></defs>` +
      '<rect width="360" height="240" fill="url(#g)"/>' +
      `<text x="20" y="214" font-family="Inter,sans-serif" font-size="18" fill="rgba(255,255,255,.92)">${label}</text></svg>`,
  )}`;

const IMG_POSTER = poster('#6549f5', '#836dff', 'Storefront option A');
const IMG_DESK = poster('#20b36b', '#0f8f86', 'Desk setup');
const IMG_VIDEO = poster('#17192f', '#777c9a', 'Launch film');

const SECTIONS = [
  ['foundations', 'Foundations'],
  ['buttons', 'Buttons'],
  ['avatars', 'Avatars'],
  ['badges', 'Badges'],
  ['inputs', 'Inputs'],
  ['controls', 'Controls'],
  ['lists', 'Lists'],
  ['feedback', 'Feedback'],
  ['messages', 'Messages'],
  ['overlays', 'Overlays'],
  ['icons', 'Icons'],
  ['mock', 'Mock scenarios'],
] as const;

const TOKENS = [
  ['--accent', 'Accent'],
  ['--accent2', 'Accent hover'],
  ['--accent-soft', 'Accent soft'],
  ['--surface', 'Surface'],
  ['--surface-2', 'Surface raised'],
  ['--surface-3', 'Surface sunken'],
  ['--text', 'Text'],
  ['--muted', 'Muted'],
  ['--line', 'Hairline'],
  ['--green', 'Success'],
  ['--danger', 'Danger'],
] as const;

const SCENARIOS = ['default', 'empty', 'scanning', 'no-network', 'firewall', 'reconnecting', 'offline-peer', 'incoming-call', 'active-call', 'group-call', 'screen-share', 'dnd', 'dark'] as const;

function Card({ id, title, wide, children }: { id: string; title: string; wide?: boolean; children: React.ReactNode }) {
  return (
    <section className={wide ? 'gallery-card gallery-wide' : 'gallery-card'} id={id}>
      <h2>{title}</h2>
      {children}
    </section>
  );
}

export function Gallery() {
  const [dark, setDark] = useState(false);
  const [query, setQuery] = useState('');
  const [slider, setSlider] = useState(72);
  const [tab, setTab] = useState('chats');
  const [segment, setSegment] = useState('right');
  const [pick, setPick] = useState('all');
  const [notify, setNotify] = useState(true);
  const [playing, setPlaying] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [dialog, setDialog] = useState(false);
  const [toasts, setToasts] = useState<readonly ToastView[]>([]);
  const [passphrase, setPassphrase] = useState('lan-key');
  const [showPassphrase, setShowPassphrase] = useState(false);

  const deliveries: DeliveryState[] = ['sending', 'delivered', 'read'];

  const entries = useMemo<readonly ContextMenuEntry[]>(
    () =>
      contextMenuItems([
        { id: 'reply', label: 'Reply', icon: 'reply' },
        { id: 'copy', label: 'Copy text', icon: 'copy', shortcut: '⌘C' },
        { id: 'pin', label: 'Pin message', icon: 'pin', checked: true },
        { id: 'forward', label: 'Forward', icon: 'forward' },
        { id: 'edit', label: 'Edit', icon: 'pencil' },
        { id: 'delete', label: 'Delete', icon: 'trash', danger: true },
      ]),
    [],
  );

  const pushToast = (toast: Omit<ToastView, 'id'>) =>
    setToasts((current) => [...current.slice(-2), { ...toast, id: `${Date.now()}` }]);

  return (
    <div className={dark ? 'gallery dark' : 'gallery'}>
      <header className="gallery-head">
        <div>
          <h1>AlverChat component gallery</h1>
          <p>Every primitive in <span className="gallery-code">packages/ui/src/components</span> rendered from the shared tokens. Dev-only screen — the shipped app never mounts it.</p>
        </div>
        <Segmented
          label="Theme"
          value={dark ? 'dark' : 'light'}
          onChange={(next) => setDark(next === 'dark')}
          options={[{ value: 'light', label: 'Light', icon: 'sun' }, { value: 'dark', label: 'Dark', icon: 'circle' }]}
        />
        <Button variant="secondary" icon="settings" onClick={() => setDialog(true)}>
          Dialog example
        </Button>
      </header>

      <nav className="gallery-nav" aria-label="Gallery sections">
        {SECTIONS.map(([id, label]) => <a key={id} href={`#${id}`}>{label}</a>)}
      </nav>

      <div className="gallery-grid">
        <Card id="foundations" title="Foundations" wide>
          <div className="gallery-row">
            {TOKENS.map(([token, label]) => (
              <div key={token} style={{ width: 116 }}>
                <div className="gallery-swatch" style={{ background: `var(${token})` }} />
                <p className="gallery-code">{token}</p>
                <p className="gallery-note">{label}</p>
              </div>
            ))}
          </div>
          <p className="gallery-note">Type scale: 26/15/13/12 px with 0.08 em section labels, Inter throughout.</p>
        </Card>

        <Card id="buttons" title="Buttons">
          <div className="gallery-row">
            <Button>Send invite</Button>
            <Button variant="secondary">Cancel</Button>
            <Button variant="danger">Delete chat</Button>
            <Button variant="ghost">Reset</Button>
          </div>
          <div className="gallery-row">
            <Button size="sm" icon="plus">Small</Button>
            <Button size="md" icon="send">Medium</Button>
            <Button size="lg" icon="upload">Large</Button>
            <Button loading>Saving</Button>
            <Button disabled>Disabled</Button>
          </div>
          <div className="gallery-row">
            <IconButton name="search" label="Search" />
            <IconButton name="bell" label="Notifications" variant="soft" active />
            <IconButton name="phone" label="Call" variant="raised" />
            <IconButton name="trash" label="Remove" variant="danger" />
            <Tooltip content="Muted by default"><IconButton name="volumeX" label="Mute" /></Tooltip>
          </div>
        </Card>

        <Card id="avatars" title="Avatars">
          <div className="gallery-row">
            {AVATAR_PALETTE.map((color) => <Avatar key={color} name={`${color} sample`} color={color} />)}
          </div>
          <div className="gallery-row">
            {PRESENCE_STATES.map((presence) => <Avatar key={presence} name={presence} presence={presence} />)}
            <Avatar group name="Design Team" />
            <Avatar group glyph="megaphone" name="Everyone" size={56} />
          </div>
          <div className="gallery-row">
            <Avatar size={24} name="Small" />
            <Avatar size={40} name="Default" />
            <Avatar size={56} name="Large" />
            <Avatar size="lg" name="Profile" />
            <Avatar
              members={[
                { name: 'Anu M', color: 'cyan' },
                { name: 'Arjun Kumar', color: 'blue' },
                { name: 'Deepa R', color: 'yellow', presence: 'busy' },
                { name: 'Fathima P', color: 'rose' },
              ]}
              size={56}
            />
            <Avatar src="data:image/png;base64,cGxhY2Vob2xkZXI=" name="Photo fallback" />
          </div>
        </Card>

        <Card id="badges" title="Badges">
          <div className="gallery-row">
            <Badge>12</Badge>
            <Badge tone="accent">New</Badge>
            <Badge tone="success">Online</Badge>
            <Badge tone="danger">Blocked</Badge>
            <Badge tone="warning">Away</Badge>
            <Badge tone="outline">Draft</Badge>
            <Badge count={148} max={99} />
            <Badge dot tone="success" label="Online" />
            <Badge dot pulse tone="danger" label="Missed call" />
          </div>
          <div className="gallery-row">
            <BadgeCount count={3}><Icon name="messageCircle" size={22} /></BadgeCount>
            <BadgeCount count={120} max={99} pulse><Icon name="bell" size={22} /></BadgeCount>
          </div>
        </Card>

        <Card id="inputs" title="Inputs">
          <Input label="Display name" defaultValue="Muhammad Anshad" helper="Visible to peers on your network." />
          <Input label="Search" icon="search" placeholder="Search people" />
          <Input label="LAN passphrase" type={showPassphrase ? 'text' : 'password'} value={passphrase} onChange={(event) => setPassphrase(event.target.value)} trailingIcon={showPassphrase ? 'eyeOff' : 'eye'} trailingLabel={showPassphrase ? 'Hide passphrase' : 'Show passphrase'} onTrailingClick={() => setShowPassphrase((v) => !v)} />
          <Input label="Work email" defaultValue="not-an-email" error="Enter a valid email address." />
          <Input label="Status" defaultValue="At my desk" maxLength={40} showCount />
          <SearchInput value={query} onChange={setQuery} onClear={() => setQuery('')} shortcut="⌘K" placeholder="Search chats, people…" />
        </Card>

        <Card id="controls" title="Controls">
          <ListRow title="Enable bubbles" hint="Show message previews" trailing={<Toggle label="Enable bubbles" checked={notify} onChange={setNotify} />} />
          <Slider label="Bubble opacity" value={slider} onChange={setSlider} format={(value) => `${value}%`} bubble showValue />
          <Select label="Who can see my status" value={pick} onChange={(event) => setPick(event.target.value)} options={[{ value: 'all', label: 'Everyone on the network' }, { value: 'contacts', label: 'Saved contacts' }, { value: 'none', label: 'Nobody' }]} block />
          <Segmented label="Dock side" value={segment} onChange={setSegment} options={[{ value: 'left', label: 'Left' }, { value: 'right', label: 'Right' }]} block />
          <Tabs
            value={tab}
            onChange={setTab}
            tabs={[
              { id: 'chats', label: 'Chats', icon: 'messageCircle', count: 4 },
              { id: 'groups', label: 'Groups', icon: 'users' },
              { id: 'calls', label: 'Calls', icon: 'phone' },
              { id: 'archived', label: 'Archived', disabled: true },
            ]}
          />
        </Card>

        <Card id="lists" title="Lists" wide>
          <ListGroup section="Chats">
            <ListRow leading={<Avatar name="Arjun Kumar" color="blue" presence="online" />} title="Arjun Kumar" hint="Invoice updated. Please check." trailing={<><Badge count={3} tone="accent" /><span className="gallery-code">09:41</span></>} chevron active />
            <ListRow leading={<Avatar group name="Billing Team" />} title="Billing Team" hint="Anu: Purchase entry completed." trailing={<span className="gallery-code">09:12</span>} />
            <ListRow icon="download" title="Downloads" hint="~/Downloads" chevron />
            <ListRow icon="shield" title="Security" hint="Passphrase set" trailing={<Icon name="lock" size={16} />} chevron />
            <ListRow icon="trash" title="Delete local history" trailing={<Button size="sm" variant="danger">Delete</Button>} />
          </ListGroup>
        </Card>

        <Card id="feedback" title="Feedback" wide>
          <Banner tone="info" title="AlverChat is discovering peers" message="No other peers yet on 192.168.1.0/24." icon="scan" />
          <Banner tone="warning" title="Firewall may be blocking AlverChat" message="Allow AlverChat on private networks." link={<Button size="sm" variant="secondary">Open guide</Button>} onDismiss={() => undefined} />
          <Banner tone="danger" title="LAN passphrase does not match" message="This peer was rejected." dismissLabel="Ignore" onDismiss={() => undefined} />
          <div className="gallery-row">
            <div style={{ flex: 1 }}><ProgressBar value={64} label="Invoice_ML00037.pdf" valueText="3.4 MB of 5.3 MB" /></div>
            <div style={{ flex: 1 }}><ProgressBar value={null} tone="accent" ariaLabel="Searching" /></div>
            <div style={{ flex: 1 }}><ProgressBar value={32} tone="danger" label="Transfer failed" valueText="Retry" /></div>
          </div>
          <div className="gallery-row">
            <Skeleton variant="row" />
            <Skeleton variant="bubble" />
            <Skeleton variant="text" lines={3} />
          </div>
          <div className="gallery-row">
            <EmptyState icon="users" title="No groups yet" message="Select peers on any device and start a group in one tap." actions={<Button size="sm">New group</Button>} />
            <EmptyState compact icon="search" title="No results" message="Try a different name." />
          </div>
          <div className="gallery-row">
            <Button size="sm" variant="secondary" onClick={() => pushToast({ message: 'Backup created', title: 'Success', tone: 'success' })}>Success toast</Button>
            <Button size="sm" variant="secondary" onClick={() => pushToast({ message: 'Peer rejected the handshake', tone: 'danger', action: { label: 'Retry', onClick: () => undefined } })}>Danger toast</Button>
            <Button size="sm" variant="secondary" onClick={() => pushToast({ message: 'Arjun is online', icon: 'circle' })}>Neutral toast</Button>
          </div>
        </Card>

        <Card id="messages" title="Message bubbles" wide>
          <div className="gallery-bubble-stage">
            <MessageBubble sender="Arjun Kumar" senderColor="blue" text="Invoice updated. Please check." time="09:41" delivery="read" />
            <MessageBubble grouped sender="Arjun Kumar" senderColor="blue" text="Also update the stock report today." time="09:42" />
            <MessageBubble mine text="Yes, I’ll review and confirm in 10 minutes." time="09:43" delivery="read" />
            <MessageBubble mine text="Pending copy still on the wire." time="09:44" delivery="sending" />
            <MessageBubble mine text="Noted. I’ll update it." time="09:45" delivery="delivered" edited />
            <MessageBubble
              mine
              variant="file"
              fileName="Invoice_ML00037.pdf"
              fileSize={5_557_322}
              transfer="sending"
              transferred={3_565_158}
              totalSize={5_557_322}
              time="09:46"
              onPause={() => undefined}
              onCancel={() => undefined}
            />
            <MessageBubble variant="file" fileName="Stock_Report.xlsx" fileSize={1_248_902} transfer="paused" transferred={402_112} totalSize={1_248_902} time="09:47" onResume={() => undefined} onCancel={() => undefined} />
            <MessageBubble variant="file" fileName="design-handoff.zip" fileSize={84_210_000} transfer="failed" time="09:48" onRetry={() => undefined} onCancel={() => undefined} />
            <MessageBubble variant="file" fileName="poster-final.pdf" fileSize={2_411_752} transfer="complete" time="09:49" onDownload={() => undefined} />
            <MessageBubble variant="image" media={[{ src: IMG_POSTER, alt: 'Poster review' }, { src: IMG_DESK, alt: 'Desk setup' }]} text="Two options for the storefront." time="09:50" onOpenMedia={() => undefined} />
            <MessageBubble variant="video" media={[{ src: IMG_VIDEO, alt: 'Launch video', duration: '1:48' }]} time="09:51" onOpenMedia={() => undefined} />
            <MessageBubble
              variant="voice"
              voiceSeconds={34}
              waveformSeed="voice-34"
              playing={playing}
              playedSeconds={playing ? 12 : 0}
              onTogglePlay={() => setPlaying((value) => !value)}
              onSeek={() => undefined}
              time="09:52"
            />
            <MessageBubble text="Original message" time="09:53" reply={{ sender: 'Fathima P', text: 'Can you share the stock report?' }} />
            <MessageBubble
              text="Looks great!"
              time="09:54"
              reactions={[{ emoji: '👍', by: 'Anu M' }, { emoji: '👍', by: 'Arjun Kumar' }, { emoji: '🔥', by: 'Deepa R' }]}
              onReaction={(emoji) => pushToast({ message: `Reacted ${emoji}` })}
            />
            <div className="gallery-row">
              {deliveries.map((state) => <MessageBubble key={state} mine text={`Tick state: ${state}`} time="09:55" delivery={state} />)}
            </div>
            <MessageBubble variant="deleted" text="This message was deleted" time="09:55" mine />
            <ContextMenuTriggerRegion onOpen={setMenu}>
              <MessageBubble text="Right-click me for the message menu." time="09:56" />
            </ContextMenuTriggerRegion>
          </div>
        </Card>

        <Card id="overlays" title="Overlays">
          <Waveform seed={42} progress={playing ? 0.55 : 0.4} positionSeconds={playing ? 19 : 14} durationSeconds={34} playing={playing} onSeek={() => setPlaying(v => !v)} />
          <div className="gallery-row">
            <Tooltip content="Copied to clipboard" side="top"><Button variant="secondary" size="sm">Hover me</Button></Tooltip>
            <Badge tone="accent">v0.1.2</Badge>
          </div>
          <div className="gallery-row">
            <Button variant="secondary" onClick={() => setMenu({ x: 120, y: 220 })}>Open context menu</Button>
            <Button variant="secondary" onClick={() => pushToast({ message: 'Restored from backup', tone: 'accent' })}>Queue toast</Button>
          </div>
        </Card>

        <Card id="icons" title="Icon set" wide>
          <div className="gallery-row">
            {ICON_NAMES.map((name) => (
              <Tooltip key={name} content={name}>
                <span style={{ display: 'grid', justifyItems: 'center', gap: 4, width: 62 }}>
                  <Icon name={name} size={20} />
                  <span className="gallery-code">{name}</span>
                </span>
              </Tooltip>
            ))}
          </div>
        </Card>

        <Card id="mock" title="Mock scenarios" wide>
          <p className="gallery-note">Append <span className="gallery-code">?mock=1&amp;scenario=name</span> to any route — the whole app renders from the mock bridge with no LAN.</p>
          <div className="gallery-row">
            {SCENARIOS.map((scenario) => (
              <a key={scenario} href={`/?mock=1&scenario=${scenario}#/gallery`}><Button size="sm" variant="secondary">{scenario}</Button></a>
            ))}
          </div>
          <div className="gallery-row">
            <a href="/?mock=1#/gallery"><Button size="sm">Gallery in mock mode</Button></a>
            <a href="/?mock=1"><Button size="sm" variant="secondary">Full app in mock mode</Button></a>
            <a href="/?mock=1&scenario=incoming-call"><Button size="sm" variant="secondary">Incoming call</Button></a>
            <a href="/?mock=1&scenario=dark"><Button size="sm" variant="secondary">Dark scenario</Button></a>
          </div>
        </Card>
      </div>

      <Dialog
        open={dialog}
        onOpenChange={setDialog}
        title="Start a group"
        description="Pick the people who should be in this group. Everyone has to be on your network."
        footer={<><Button variant="ghost" onClick={() => setDialog(false)}>Cancel</Button><Button>Create group</Button></>}
      >
        <div className="gallery-stack">
          <ListRow leading={<Avatar name="Anu M" color="cyan" presence="online" />} title="Anu M" trailing={<Icon name="check" size={16} />} />
          <ListRow leading={<Avatar name="Arjun Kumar" color="blue" presence="online" />} title="Arjun Kumar" trailing={<Icon name="check" size={16} />} />
          <ListRow leading={<Avatar name="Deepa R" color="yellow" presence="busy" />} title="Deepa R" />
        </div>
      </Dialog>

      <ContextMenu open={menu !== null} onClose={() => setMenu(null)} anchor={menu} items={entries} label="Message actions" reactions={['👍', '❤️', '😂', '🎉']} onReaction={(emoji) => { setMenu(null); pushToast({ message: `Reacted ${emoji}` }); }} />

            <ToastViewport toasts={toasts} onDismiss={(id) => setToasts((current) => current.filter((toast) => toast.id !== id))} />
    </div>
  );
}
