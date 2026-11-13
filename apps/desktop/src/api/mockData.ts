import type { Account, ConnectionStatus, Label, MessageDetail } from './types';

const MB = 1024 * 1024;

export const MOCK_ACCOUNTS: readonly Account[] = [
  { id: 'work', label: 'Work', address: 'you@work.example', hue: 'blue', maxMessageBytes: 25 * MB },
  { id: 'personal', label: 'Personal', address: 'you@personal.example', hue: 'teal', maxMessageBytes: 25 * MB },
];

export const MOCK_LABELS: readonly Label[] = [
  { id: 'clients', name: 'Clients', hue: 'orange' },
  { id: 'invoices', name: 'Invoices', hue: 'purple' },
  { id: 'travel', name: 'Travel', hue: 'green' },
];

export const MOCK_CONNECTION: ConnectionStatus = {
  state: 'weak',
  lastSyncAt: '2026-11-13T07:02:00Z',
};

const text = (...paragraphs: string[]) => ({ kind: 'text' as const, text: paragraphs.join('\n\n') });

/** Sample mailbox matching the published design. Times are UTC (Nairobi is UTC+3). */
export const MOCK_MESSAGES: readonly MessageDetail[] = [
  {
    id: 'm1', folder: 'inbox', accountId: 'work', direction: 'incoming',
    counterpart: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' },
    subject: 'Revised quotation for the Westlands office fit-out',
    preview: 'Attached is the revised quotation with the changes we discussed on Tuesday.',
    date: '2026-11-13T06:42:00Z', unread: true, flagged: false, hasAttachments: true, labelIds: ['clients'],
    body: text(
      'Hi,',
      'Attached is the revised quotation with the changes we discussed on Tuesday. The glass partitions are now frosted on the lower half, and the electrical work has its own line so you can approve it separately.',
      'If you are happy with it, sign page 3 and send it back. We can start on the 24th.',
      'Thanks,\nWanjiru',
    ),
    attachments: [
      { name: 'Quotation-Westlands-rev2.pdf', mimeType: 'application/pdf', sizeBytes: 412 * 1024 },
      { name: 'Floor-plan.png', mimeType: 'image/png', sizeBytes: Math.round(1.8 * MB) },
    ],
  },
  {
    id: 'm2', folder: 'inbox', accountId: 'work', direction: 'incoming',
    counterpart: { name: 'Build server', address: 'builds@ci.example' },
    subject: 'Nightly build passed on main',
    preview: 'All tests passed. Linux, Windows and macOS installers are ready to download.',
    date: '2026-11-13T06:15:00Z', unread: true, flagged: false, hasAttachments: false, labelIds: [],
    body: {
      kind: 'html',
      html: '<p>The nightly build of <b>main</b> finished without errors.</p><p>All tests passed. Linux (deb, rpm, AppImage, Flatpak), Windows and macOS installers are ready on the build page.</p>',
    },
    attachments: [],
  },
  {
    id: 'm3', folder: 'inbox', accountId: 'personal', direction: 'incoming',
    counterpart: { name: 'Brian Otieno', address: 'brian@personal.example' },
    subject: 'Lunch on Friday?',
    preview: 'Are you free around 1? There is a new place near Yaya Centre I want to try.',
    date: '2026-11-13T05:58:00Z', unread: true, flagged: false, hasAttachments: false, labelIds: [],
    body: text('Hey,', 'Are you free around 1 on Friday? There is a new place near Yaya Centre I want to try. My treat if you are buying the coffee after.', 'Brian'),
    attachments: [],
  },
  {
    id: 'm4', folder: 'inbox', accountId: 'work', direction: 'incoming',
    counterpart: { name: 'Amina Hassan', address: 'amina@studio.example' },
    subject: 'Logo files, final round',
    preview: 'Here are the final logo files in SVG, PNG and PDF, plus the white version.',
    date: '2026-11-12T13:20:00Z', unread: false, flagged: true, hasAttachments: true, labelIds: ['clients'],
    body: text(
      'Hi,',
      'Here are the final logo files in SVG, PNG and PDF. I have included the white version for dark backgrounds and a one-colour version for printing.',
      'Let me know if anything needs a last tweak before we hand over.',
      'Amina',
    ),
    attachments: [{ name: 'Logo-final.zip', mimeType: 'application/zip', sizeBytes: Math.round(6.4 * MB) }],
  },
  {
    id: 'm5', folder: 'inbox', accountId: 'work', direction: 'incoming',
    counterpart: { name: 'Calendar', address: 'calendar@work.example' },
    subject: 'Invitation: Sprint review, Fri 14:00',
    preview: 'Sprint review, Friday 14:00 to 15:00, Boardroom.',
    date: '2026-11-12T08:03:00Z', unread: true, flagged: false, hasAttachments: false, labelIds: [],
    body: text('You have been invited to Sprint review.', 'Friday 13 November, 14:00 to 15:00, Boardroom.'),
    attachments: [],
  },
  {
    id: 'm6', folder: 'inbox', accountId: 'work', direction: 'incoming',
    counterpart: { name: 'Peter Mwangi', address: 'peter@client.example' },
    subject: 'Invoice INV-0142 paid',
    preview: 'Payment received, thank you. Receipt attached for your records.',
    date: '2026-11-11T12:47:00Z', unread: false, flagged: false, hasAttachments: true, labelIds: ['invoices'],
    body: text('Hello,', 'Payment for INV-0142 has gone through. The receipt is attached for your records.', 'Regards,\nPeter'),
    attachments: [{ name: 'Receipt-INV-0142.pdf', mimeType: 'application/pdf', sizeBytes: 96 * 1024 }],
  },
  {
    id: 'm7', folder: 'inbox', accountId: 'personal', direction: 'incoming',
    counterpart: { name: 'Grace Njeri', address: 'grace@personal.example' },
    subject: 'Photos from the weekend',
    preview: 'Finally sorted through them. The ones from Naivasha came out really well.',
    date: '2026-11-10T17:12:00Z', unread: false, flagged: false, hasAttachments: true, labelIds: ['travel'],
    body: text('Finally sorted through them. The ones from Naivasha came out really well, especially the sunset ones.', 'A few favourites are attached.'),
    attachments: [{ name: 'Naivasha-favourites.zip', mimeType: 'application/zip', sizeBytes: Math.round(18.2 * MB) }],
  },
  {
    id: 'm8', folder: 'inbox', accountId: 'work', direction: 'incoming',
    counterpart: { name: 'Project updates', address: 'updates@mocky.example' },
    subject: 'Release notes for version 0.9',
    preview: 'Faster search, a new offline queue and better handling of large mailboxes.',
    date: '2026-11-03T05:00:00Z', unread: false, flagged: false, hasAttachments: false, labelIds: [],
    body: text('Version 0.9 is out.', 'Search is much faster on large mailboxes, the new Outbox queues mail when you are offline, and attachments over your provider limit are caught before sending.'),
    attachments: [],
  },
  {
    id: 'o1', folder: 'outbox', accountId: 'work', direction: 'outgoing',
    counterpart: { name: 'Wanjiru Kamau', address: 'wanjiru@fitout.example' },
    subject: 'Re: Revised quotation for the Westlands office fit-out',
    preview: 'Thanks Wanjiru, signed copy attached. Let us go ahead with the 24th.',
    date: '2026-11-13T07:05:00Z', unread: false, flagged: false, hasAttachments: true, labelIds: [],
    outbox: { kind: 'retrying', retryInSeconds: 12 },
    body: text('Thanks Wanjiru,', 'Signed copy attached. Let us go ahead with the 24th.'),
    attachments: [{ name: 'Quotation-signed.pdf', mimeType: 'application/pdf', sizeBytes: 520 * 1024 }],
  },
  {
    id: 'o2', folder: 'outbox', accountId: 'work', direction: 'outgoing',
    counterpart: { name: 'Amina Hassan', address: 'amina@studio.example' },
    subject: 'Re: Logo files, final round',
    preview: 'These look great. One small change: can we get the mark on its own as an app icon?',
    date: '2026-11-13T07:07:00Z', unread: false, flagged: false, hasAttachments: false, labelIds: [],
    outbox: { kind: 'sending', progress: 64 },
    body: text('These look great.', 'One small change: can we get the mark on its own as an app icon, 1024 px square?'),
    attachments: [],
  },
  {
    id: 'o3', folder: 'outbox', accountId: 'work', direction: 'outgoing',
    counterpart: { name: 'Client accounts', address: 'accounts@client.example' },
    subject: 'Invoice INV-0143',
    preview: 'Please find attached invoice INV-0143 for October, with the signed delivery notes.',
    date: '2026-11-13T07:11:00Z', unread: false, flagged: false, hasAttachments: true, labelIds: ['invoices'],
    outbox: { kind: 'failed', reason: 'too_large', estimatedBytes: 28 * MB, limitBytes: 25 * MB },
    body: text('Hello,', 'Please find attached invoice INV-0143 for October, together with the signed delivery notes.'),
    attachments: [
      { name: 'INV-0143.pdf', mimeType: 'application/pdf', sizeBytes: 180 * 1024 },
      { name: 'Delivery-notes-scanned.pdf', mimeType: 'application/pdf', sizeBytes: Math.round(27.8 * MB) },
    ],
  },
  {
    id: 'd1', folder: 'drafts', accountId: 'personal', direction: 'outgoing',
    counterpart: { name: 'Brian Otieno', address: 'brian@personal.example' },
    subject: 'Re: Lunch on Friday?', preview: 'Friday works. 1pm at',
    date: '2026-11-13T07:14:00Z', unread: false, flagged: false, hasAttachments: false, labelIds: [],
    body: text('Friday works. 1pm at'), attachments: [],
  },
  {
    id: 's1', folder: 'sent', accountId: 'work', direction: 'outgoing',
    counterpart: { name: 'Peter Mwangi', address: 'peter@client.example' },
    subject: 'Invoice INV-0142', preview: 'Hi Peter, invoice INV-0142 for September is attached.',
    date: '2026-11-11T06:30:00Z', unread: false, flagged: false, hasAttachments: true, labelIds: ['invoices'],
    body: text('Hi Peter,', 'Invoice INV-0142 for September is attached.'),
    attachments: [{ name: 'INV-0142.pdf', mimeType: 'application/pdf', sizeBytes: 176 * 1024 }],
  },
  {
    id: 's2', folder: 'sent', accountId: 'personal', direction: 'outgoing',
    counterpart: { name: 'Grace Njeri', address: 'grace@personal.example' },
    subject: 'Re: Photos from the weekend', preview: 'These are lovely. Print the sunset one for me?',
    date: '2026-11-10T18:02:00Z', unread: false, flagged: false, hasAttachments: false, labelIds: [],
    body: text('These are lovely. Print the sunset one for me?'), attachments: [],
  },
];
