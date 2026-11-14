import type { Account, Folder, FolderRole, OutboxEntry, Settings } from '../types';
import type { MockMessage } from './mockMailApi';

const MB = 1024 * 1024;
/** Unix seconds for an ISO time. */
export const at = (iso: string): number => Math.floor(Date.parse(iso) / 1000);

/** "Now" for the sample data: Fri 13 Nov 2026, 10:15 in Nairobi. */
export const MOCK_NOW = at('2026-11-13T07:15:00Z');

export const MOCK_ACCOUNTS: Account[] = [
  {
    id: 'work',
    email: 'you@work.example',
    displayName: 'Work',
    auth: 'password',
    createdAt: at('2026-10-01T08:00:00Z'),
    color: '#2450C8',
    imap: { host: 'imap.work.example', port: 993, security: 'tls' },
    smtp: { host: 'smtp.work.example', port: 465, security: 'tls' },
    sizeLimitBytes: 25 * MB,
  },
  {
    id: 'personal',
    email: 'you@personal.example',
    displayName: 'Personal',
    auth: 'password',
    createdAt: at('2026-10-02T08:00:00Z'),
    color: '#0F7B6C',
    imap: { host: 'imap.personal.example', port: 993, security: 'tls' },
    smtp: { host: 'smtp.personal.example', port: 587, security: 'starttls' },
    sizeLimitBytes: 20 * MB,
  },
];

const ROLE_FOLDERS: [FolderRole, string][] = [
  ['inbox', 'Inbox'],
  ['drafts', 'Drafts'],
  ['sent', 'Sent'],
  ['archive', 'Archive'],
  ['junk', 'Spam'],
  ['trash', 'Trash'],
];

/** Folders without counts; the mock computes unread and total from messages. */
export function foldersFor(accountId: string, extra: [string, string][] = []): Omit<Folder, 'unread' | 'total'>[] {
  return [
    ...ROLE_FOLDERS.map(([role, name]) => ({ id: `${accountId}:${role}`, accountId, name, path: name, role })),
    ...extra.map(([name, path]) => ({
      id: `${accountId}:${path}`,
      accountId,
      name,
      path,
      role: 'other' as const,
    })),
  ];
}

export const MOCK_FOLDERS = [
  ...foldersFor('work', [
    ['Clients', 'Clients'],
    ['Résumés', 'R&AMk-sum&AOk-s'],
  ]),
  ...foldersFor('personal', [['Travel', 'Travel']]),
];

const me = (accountId: string) => ({
  name: null,
  address: accountId === 'work' ? 'you@work.example' : 'you@personal.example',
});

function message(m: Partial<MockMessage> & Pick<MockMessage, 'id' | 'accountId' | 'folderId' | 'subject'>): MockMessage {
  return {
    from: null,
    to: [me(m.accountId)],
    preview: '',
    date: null,
    unread: false,
    flagged: false,
    answered: false,
    hasAttachments: (m.attachments?.length ?? 0) > 0,
    cc: [],
    replyTo: [],
    html: null,
    text: null,
    blockedRemoteImages: 0,
    attachments: [],
    messageIdHeader: `<${m.id}@mock.example>`,
    references: [],
    ...m,
  };
}

const para = (...p: string[]) => p.join('\n\n');

export const MOCK_MESSAGES: MockMessage[] = [
  message({
    id: 'm1', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' },
    cc: [{ name: 'Amina Hassan', address: 'amina@studio.example' }, { name: null, address: 'you@work.example' }],
    subject: 'Revised quotation for the Westlands office fit-out',
    preview: 'Attached is the revised quotation with the changes we discussed on Tuesday.',
    date: at('2026-11-13T06:42:00Z'), unread: true,
    text: para(
      'Hi,',
      'Attached is the revised quotation with the changes we discussed on Tuesday. The glass partitions are now frosted on the lower half, and the electrical work has its own line so you can approve it separately.',
      'Details are on https://fitout.example/q/2041 if you prefer to read it online.',
      'Thanks,\nWanjiru',
    ),
    attachments: [
      { index: 0, fileName: 'Quotation-Westlands-rev2.pdf', mimeType: 'application/pdf', size: 412 * 1024 },
      { index: 1, fileName: 'Floor-plan.png', mimeType: 'image/png', size: Math.round(1.8 * MB) },
    ],
    references: ['<q-2040@fitout.example>'],
  }),
  message({
    id: 'm2', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Build server', address: 'builds@ci.example' },
    subject: 'Nightly build passed on main',
    preview: 'All tests passed. Linux, Windows and macOS installers are ready to download.',
    date: at('2026-11-13T06:15:00Z'), unread: true,
    html: '<p>The nightly build of <b>main</b> finished without errors.</p><p>Installers are ready.</p>',
    remoteHtml:
      '<p>The nightly build of <b>main</b> finished without errors.</p><img src="https://ci.example/badge.svg" alt="Build passing"><p>Installers are ready.</p>',
    blockedRemoteImages: 1,
  }),
  message({
    id: 'm3', accountId: 'personal', folderId: 'personal:inbox',
    from: { name: 'Brian Otieno', address: 'brian@personal.example' },
    replyTo: [{ name: 'Brian (phone)', address: 'brian.m@personal.example' }],
    subject: 'Lunch on Friday?',
    preview: 'Are you free around 1? There is a new place near Yaya Centre I want to try.',
    date: at('2026-11-13T05:58:00Z'), unread: true,
    text: para('Hey,', 'Are you free around 1 on Friday? There is a new place near Yaya Centre I want to try.', 'Brian'),
  }),
  message({
    id: 'm4', accountId: 'work', folderId: 'work:Clients',
    from: { name: 'Amina Hassan', address: 'amina@studio.example' },
    subject: 'Logo files, final round',
    preview: 'Here are the final logo files in SVG, PNG and PDF, plus the white version.',
    date: at('2026-11-12T13:20:00Z'), flagged: true,
    text: para('Hi,', 'Here are the final logo files in SVG, PNG and PDF.', 'Amina'),
    attachments: [{ index: 0, fileName: 'Logo-final.zip', mimeType: 'application/zip', size: Math.round(6.4 * MB) }],
  }),
  message({
    id: 'm5', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Calendar', address: 'calendar@work.example' },
    subject: 'Invitation: Sprint review, Fri 14:00',
    preview: 'Sprint review, Friday 14:00 to 15:00, Boardroom.',
    date: at('2026-11-12T08:03:00Z'), unread: true,
    text: para('You have been invited to Sprint review.', 'Friday 13 November, 14:00 to 15:00, Boardroom.'),
  }),
  message({
    id: 'm6', accountId: 'work', folderId: 'work:inbox',
    from: { name: 'Peter Mwangi', address: 'peter@client.example' },
    subject: 'Invoice INV-0142 paid',
    preview: 'Payment received, thank you. Receipt attached for your records.',
    date: at('2026-11-11T12:47:00Z'), answered: true,
    text: para('Hello,', 'Payment for INV-0142 has gone through.', 'Regards,\nPeter'),
    attachments: [{ index: 0, fileName: 'Receipt-INV-0142.pdf', mimeType: 'application/pdf', size: 96 * 1024 }],
  }),
  message({
    id: 'm7', accountId: 'personal', folderId: 'personal:inbox',
    from: { name: 'Grace Njeri', address: 'grace@personal.example' },
    subject: 'Photos from the weekend',
    preview: 'Finally sorted through them. The ones from Naivasha came out really well.',
    date: at('2026-11-10T17:12:00Z'),
    text: para('Finally sorted through them. The ones from Naivasha came out really well.'),
    attachments: [{ index: 0, fileName: 'Naivasha.zip', mimeType: 'application/zip', size: Math.round(18.2 * MB) }],
  }),
  message({
    id: 'd1', accountId: 'personal', folderId: 'personal:drafts',
    to: [{ name: 'Brian Otieno', address: 'brian@personal.example' }],
    from: { name: null, address: 'you@personal.example' },
    subject: 'Re: Lunch on Friday?', preview: 'Friday works. 1pm at',
    date: at('2026-11-13T07:14:00Z'), text: 'Friday works. 1pm at',
  }),
  message({
    id: 's1', accountId: 'work', folderId: 'work:sent',
    to: [{ name: 'Peter Mwangi', address: 'peter@client.example' }],
    from: { name: null, address: 'you@work.example' },
    subject: 'Invoice INV-0142', preview: 'Hi Peter, invoice INV-0142 for September is attached.',
    date: at('2026-11-11T06:30:00Z'), text: para('Hi Peter,', 'Invoice INV-0142 for September is attached.'),
  }),
];

export const MOCK_OUTBOX: OutboxEntry[] = [
  {
    id: 'o1', accountId: 'work', to: [{ name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' }],
    subject: 'Re: Revised quotation for the Westlands office fit-out',
    preview: 'Thanks Wanjiru, signed copy attached. Let us go ahead with the 24th.',
    createdAt: at('2026-11-13T07:05:00Z'), attempts: 2,
    status: { state: 'queued', next_attempt_at: MOCK_NOW + 12, maybe_sent: false }, progress: null,
  },
  {
    id: 'o2', accountId: 'work', to: [{ name: 'Amina Hassan', address: 'amina@studio.example' }],
    subject: 'Re: Logo files, final round',
    preview: 'These look great. Can we get the mark on its own as an app icon?',
    createdAt: at('2026-11-13T07:07:00Z'), attempts: 1,
    status: { state: 'sending', started_at: MOCK_NOW - 3 }, progress: 0.64,
  },
  {
    id: 'o3', accountId: 'work', to: [{ name: 'Client accounts', address: 'accounts@client.example' }],
    subject: 'Invoice INV-0143',
    preview: 'Please find attached invoice INV-0143 for October, with the signed delivery notes.',
    createdAt: at('2026-11-13T07:11:00Z'), attempts: 0,
    status: {
      state: 'failed', reason: 'too_large',
      message: 'message is about 29360128 bytes once encoded; the server accepts 26214400',
    },
    progress: null,
  },
];

export const MOCK_SETTINGS: Settings = { theme: 'system', remoteImages: 'block', notifications: true };
