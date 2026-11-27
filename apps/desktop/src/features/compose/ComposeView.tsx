import { useEffect, useId, useRef, useState, type KeyboardEvent, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { errorTitle, toAppError } from '../../api/errors';
import type {
  Account,
  AppError,
  Draft,
  DraftAttachment,
  Importance,
  Mailbox,
  OutboxEntry,
  RecipientCheck,
  SecurityKind,
  SignatureImage,
  SignatureSet,
} from '../../api/types';
import { formatBytes } from '../../lib/format';
import { useDebounced } from '../../lib/useDebounced';
import { kindLabel, kindName, kindNoun } from '../security/securityText';
import { Icon } from '../../ui/Icon';
import { MenuButton, MenuGroup, MenuItem, MenuSeparator } from '../../ui/Menu';
import { Modal } from '../../ui/Modal';
import { RichEditorSlot, type EditorHandle } from './editor/loader';
import { ContactCardDialog, ItemDialog, NameChooser, SymbolDialog } from './InsertDialogs';
import { RecipientField } from './RecipientField';
import { entryFor, parseRecipients, splitRecipients } from './recipients';
import { htmlToPlainText, swapSignatureHtml, textToHtml } from './richText';
import { signatureBlock, signatureFor, type ComposeKind } from './seed';
import { MissingKeysDialog } from './MissingKeysDialog';
import { kindFor, recipientAddresses, withoutRecipients, type AccountSecurity } from './security';

const IMPORTANCE: readonly [Importance, string, string][] = [
  ['high', 'High', 'High importance'],
  ['normal', 'Normal', 'Normal importance'],
  ['low', 'Low', 'Low importance'],
];

interface Props {
  initial: Draft;
  /** Load copies of this message's attachments into the draft (forwarding, continuing a draft). */
  attachmentsFrom?: string | null;
  title: string;
  accounts: readonly Account[];
  offlineNote: boolean;
  onClose: () => void;
  onSent: (entry: OutboxEntry) => void;
  /**
   * What is being written. New messages, replies and forwards get the From
   * account's signature (swapped with the account); a draft is left as it is.
   */
  kind?: ComposeKind;
  /**
   * The message exists nowhere else (it was taken back out of the Outbox),
   * so closing asks first even when nothing was typed.
   */
  unsaved?: boolean;
  /**
   * Lets the shell leave compose safely. While the message has unsaved text
   * this hands the shell a function to call with what it wants to do next;
   * that runs after the user's answer (save or discard). Null: nothing to ask.
   */
  onLeaveGuard?: (ask: LeaveCompose | null) => void;
  /**
   * Computers: offer Pop out, which carries on with the message in a window
   * of its own. Called once that window is open; compose here then closes.
   */
  onPoppedOut?: (() => void) | undefined;
  /** Phone layout: the editor's toolbar is one scrolling row. */
  phone?: boolean;
  /** A window of its own: compose fills it. */
  standalone?: boolean;
}

/** Ask compose to close, then carry on with `then`. */
export type LeaveCompose = (then: () => void) => void;

/** Wait this long after the recipients stop changing before asking which can get encrypted mail. */
const ENCRYPT_CHECK_MS = 400;

type FieldName = 'to' | 'cc' | 'bcc';
const FIELDS: readonly FieldName[] = ['to', 'cc', 'bcc'];

function recipientError(input: string, required: boolean): string | null {
  const { valid, invalid } = parseRecipients(input);
  if (invalid.length > 0) return `Check ${invalid.join(', ')}. That does not look like an email address.`;
  if (required && valid.length === 0) return 'Add at least one recipient.';
  return null;
}

/** Rough encoded size of the attachments, for the hint only; the backend checks for real. */
function encodedEstimate(bytes: number): number {
  return Math.ceil(bytes / 3) * 4;
}

/** The message as the editor holds it: its HTML, or HTML made from plain text (older drafts). */
function htmlOf(draft: Draft): string {
  return draft.bodyHtml ?? textToHtml(draft.bodyText);
}

/** The body replaced, with its plain-text version kept in step. */
function withBody(draft: Draft, html: string, text: string = htmlToPlainText(html)): Draft {
  return { ...draft, bodyHtml: html, bodyText: text };
}

/**
 * Where the message goes while the editor loads (the first time only). It
 * holds the focus a reply starts with, so Esc and Ctrl+Enter work meanwhile.
 */
function EditorLoading({ focus }: { focus: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (focus) ref.current?.focus();
  }, [focus]);
  return (
    <div ref={ref} className="compose__body compose__body--loading" aria-busy="true" tabIndex={-1}>
      Getting the editor ready
    </div>
  );
}

/** A typed name Check Names found more than one contact for. */
interface Choice {
  field: FieldName;
  input: string;
  candidates: Mailbox[];
}

export function ComposeView({
  initial,
  attachmentsFrom = null,
  title,
  accounts,
  offlineNote,
  onClose,
  onSent,
  kind = 'draft',
  unsaved: startsUnsaved = false,
  onLeaveGuard,
  onPoppedOut,
  phone = false,
  standalone = false,
}: Props) {
  const api = useMailApi();
  const id = useId();
  const editorRef = useRef<EditorHandle>(null);
  const [draft, setDraft] = useState<Draft>(initial);
  /** The last saved (or loaded) state; changes are compared against it. */
  const [baseline, setBaseline] = useState<Draft>(initial);
  const [loadingAttachments, setLoadingAttachments] = useState(attachmentsFrom !== null);
  const [attachError, setAttachError] = useState<AppError | null>(null);
  const [showCc, setShowCc] = useState(initial.cc !== '');
  const [showBcc, setShowBcc] = useState(initial.bcc !== '');
  const [fieldErrors, setFieldErrors] = useState<Partial<Record<FieldName, string>>>({});
  /** Bumped when Check Names rewrites the fields, so they show the new entries. */
  const [fieldsRevision, setFieldsRevision] = useState(0);
  const [error, setError] = useState<AppError | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState<null | 'send' | 'save' | 'attach' | 'check' | 'popout'>(null);
  const [confirmClose, setConfirmClose] = useState(false);
  const [unsaved, setUnsaved] = useState(startsUnsaved);
  /** Where the shell wants to go once this message is saved or discarded. */
  const [afterClose, setAfterClose] = useState<(() => void) | null>(null);
  const [inserting, setInserting] = useState<null | 'symbol' | 'contact' | 'item'>(null);
  /** Check Names: names still to pick a contact for, and whether Send waits on them. */
  const [choices, setChoices] = useState<Choice[]>([]);
  const [sendAfterChoices, setSendAfterChoices] = useState(false);

  // Signing and encryption: what the From account has, and the defaults from Settings › Security.
  const [security, setSecurity] = useState<AccountSecurity | null>(null);
  const [prefs, setPrefs] = useState<{ sign: boolean; encrypt: boolean } | null>(null);
  const prefsLoad = useRef<Promise<{ sign: boolean; encrypt: boolean }> | null>(null);
  const securityLoaded = useRef(false);
  /** Sign or Encrypt chosen by hand: the defaults leave it alone from then on. */
  const touched = useRef({ sign: false, encrypt: false });
  /** The last answer to "can these recipients get encrypted mail?". */
  const [encryptCheck, setEncryptCheck] = useState<{ missing: string[]; ownKey: boolean } | null>(null);
  /** Send found recipients without a key: ask what to do. */
  const [missingAsk, setMissingAsk] = useState<{ draft: Draft; missing: string[]; kind: SecurityKind } | null>(null);

  useEffect(() => {
    let active = true;
    const accountId = draft.accountId;
    prefsLoad.current ??= api.getSettings().then(
      (s) => ({ sign: s.signByDefault, encrypt: s.encryptWhenPossible }),
      () => ({ sign: false, encrypt: false }),
    );
    void Promise.all([api.getAccountKeys(accountId).catch(() => null), prefsLoad.current]).then(([keys, defaults]) => {
      if (!active) return;
      const next: AccountSecurity = {
        accountId,
        smime: keys?.smime != null,
        openpgp: keys?.openpgp != null,
        preferred: keys?.preferred ?? 'smime',
      };
      const first = !securityLoaded.current;
      securityLoaded.current = true;
      setSecurity(next);
      setPrefs(defaults);
      // A draft carried on with keeps what it said; anything else starts from the defaults.
      const carriedOn = first && (initial.sign !== undefined || initial.encrypt !== undefined);
      const apply = (d: Draft): Draft => {
        if (d.accountId !== accountId) return d;
        const has = next[kindFor(next, d.securityKind)];
        if (!has) {
          // Nothing to sign or encrypt with from this account.
          return d.sign === true || d.encrypt === true ? { ...d, sign: false, encrypt: false } : d;
        }
        if (!carriedOn && !touched.current.sign && defaults.sign && d.sign !== true) return { ...d, sign: true };
        return d;
      };
      setDraft(apply);
      // Defaults added as compose opens aren't changes the user made.
      if (first) setBaseline(apply);
    });
    return () => {
      active = false;
    };
    // `initial` is only read for the first account; reloading when it changes identity would start over.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, draft.accountId]);

  useEffect(() => {
    if (attachmentsFrom === null) return;
    let active = true;
    const merge = (d: Draft, files: DraftAttachment[]): Draft => {
      const known = new Set(d.attachments.map((a) => a.path));
      return { ...d, attachments: [...d.attachments, ...files.filter((f) => !known.has(f.path))] };
    };
    api
      .attachmentsForDraft(attachmentsFrom)
      .then(
        (files) => {
          if (!active) return;
          setDraft((d) => merge(d, files));
          setBaseline((b) => merge(b, files));
        },
        (e: unknown) => {
          if (active) setAttachError(toAppError(e));
        },
      )
      .finally(() => {
        if (active) setLoadingAttachments(false);
      });
    return () => {
      active = false;
    };
  }, [api, attachmentsFrom]);

  // The signature now in the body, so changing From or the picker can swap it.
  const signature = useRef<string | null>(null);
  const signatureLoaded = useRef(false);
  // The From account's signatures, for the picker.
  const [signatures, setSignatures] = useState<SignatureSet | null>(null);
  const [signatureId, setSignatureId] = useState<string | null>(null);
  // Its picture, previewed under the body; it is added when the message is sent.
  const [signatureImage, setSignatureImage] = useState<{ text: string; image: SignatureImage } | null>(null);
  // New messages start with one default signature, replies and forwards with the other.
  const signatureKind = signatureFor(kind);

  /** Put signature `id` (null: none) in the body in place of the current one. */
  const applySignature = (set: SignatureSet, id: string | null, atStart: boolean) => {
    const next = set.items.find((s) => s.id === id) ?? null;
    const previous = signature.current;
    const text = next && next.text !== '' ? next.text : null;
    signature.current = text;
    setSignatureId(next?.id ?? null);
    setSignatureImage(next?.image && text !== null ? { text, image: next.image } : null);
    const swap = (d: Draft) => {
      const html = htmlOf(d);
      const swapped = swapSignatureHtml(html, previous, text, next?.id ?? null);
      return swapped === html ? d : withBody(d, swapped);
    };
    setDraft(swap);
    // Adding it at the start isn't a change the user made.
    if (atStart) setBaseline(swap);
  };

  useEffect(() => {
    if (signatureKind === null) return;
    let active = true;
    api.listSignatures(draft.accountId).then(
      (set) => {
        if (!active) return;
        setSignatures(set);
        applySignature(set, signatureKind === 'reply' ? set.defaultReply : set.defaultNew, !signatureLoaded.current);
        signatureLoaded.current = true;
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
    // applySignature only reads refs and setters; reloading on its identity would loop.
  }, [api, signatureKind, draft.accountId]);

  const dirty = unsaved || JSON.stringify(draft) !== JSON.stringify(baseline);
  const account = accounts.find((a) => a.id === draft.accountId);
  const attachmentBytes = draft.attachments.reduce((sum, a) => sum + a.size, 0);
  const overLimit =
    account?.sizeLimitBytes != null && encodedEstimate(attachmentBytes) > account.sizeLimitBytes;

  const securityKind = security ? kindFor(security, draft.securityKind) : (draft.securityKind ?? 'smime');
  const hasKey = security?.accountId === draft.accountId && security[securityKind];
  const bothKinds = security?.accountId === draft.accountId && security.smime && security.openpgp;
  const recipients = recipientAddresses(draft);
  const wantEncrypt = draft.encrypt === true;
  // Ask again once the recipients (or the kind) stop changing.
  const checkKey = useDebounced(
    hasKey ? [draft.accountId, securityKind, ...recipients].join('\n') : '',
    ENCRYPT_CHECK_MS,
  );
  useEffect(() => {
    const [accountId = '', kind, ...addresses] = checkKey.split('\n');
    if (accountId === '' || (kind !== 'smime' && kind !== 'openpgp') || addresses.length === 0 || !prefs) return;
    // Encryption on, or Encrypt when possible deciding whether to turn it on.
    const auto = prefs.encrypt && !touched.current.encrypt;
    if (!wantEncrypt && !auto) return;
    let active = true;
    api.checkEncryption(accountId, addresses, kind).then(
      (result) => {
        if (!active) return;
        const missing = result.recipients.filter((r) => !r.canEncrypt).map((r) => r.address);
        setEncryptCheck({ missing, ownKey: result.ownKey });
        if (!auto || touched.current.encrypt) return;
        const on = missing.length === 0 && result.ownKey;
        const follow = (d: Draft): Draft => ((d.encrypt === true) === on ? d : { ...d, encrypt: on });
        setDraft((d) => (d.accountId === accountId ? follow(d) : d));
        // Not a change the user made.
        setBaseline(follow);
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [api, checkKey, wantEncrypt, prefs]);
  const current = new Set(recipients.map((a) => a.toLowerCase()));
  const noKey = wantEncrypt && encryptCheck ? encryptCheck.missing.filter((a) => current.has(a.toLowerCase())) : [];
  const noKeySet = new Set(noKey.map((a) => a.toLowerCase()));
  const securityNote =
    !hasKey || (draft.sign !== true && !wantEncrypt)
      ? null
      : draft.sign === true && wantEncrypt
        ? `Signed and encrypted with your ${kindLabel(securityKind)}. Only the recipients can read it; the subject stays visible.`
        : draft.sign === true
          ? `Signed with your ${kindLabel(securityKind)}, so recipients can check it came from you and wasn’t changed.`
          : `Encrypted with ${kindName(securityKind)}. Only the recipients can read it; the subject stays visible.`;

  const update = (patch: Partial<Draft>) => {
    setDraft((d) => ({ ...d, ...patch }));
    setStatus(null);
  };

  const addFiles = (files: readonly DraftAttachment[]) => {
    setDraft((d) => {
      const known = new Set(d.attachments.map((a) => a.path));
      return { ...d, attachments: [...d.attachments, ...files.filter((f) => !known.has(f.path))] };
    });
  };

  const validate = (d: Draft): boolean => {
    const errors: Partial<Record<FieldName, string>> = {};
    const to = recipientError(d.to, d.cc.trim() === '' && d.bcc.trim() === '');
    const cc = recipientError(d.cc, false);
    const bcc = recipientError(d.bcc, false);
    if (to) errors.to = to;
    if (cc) errors.cc = cc;
    if (bcc) errors.bcc = bcc;
    setFieldErrors(errors);
    return Object.keys(errors).length === 0;
  };

  const sendNow = (d: Draft) => {
    if (!validate(d)) return;
    setBusy('send');
    // Signed or encrypted: say with which kind, so the backend doesn't have to guess.
    const kind = security ? kindFor(security, d.securityKind) : d.securityKind;
    const final: Draft = (d.sign === true || d.encrypt === true) && kind !== undefined ? { ...d, securityKind: kind } : d;
    // Encrypted: everyone needs a key, checked now rather than trusting the last look.
    const missing: Promise<string[]> =
      final.encrypt === true && kind !== undefined
        ? api
            .checkEncryption(final.accountId, recipientAddresses(final), kind)
            .then((r) => r.recipients.filter((x) => !x.canEncrypt).map((x) => x.address))
        : Promise.resolve([]);
    missing
      .then((without) => {
        if (without.length > 0 && kind !== undefined) {
          setBusy(null);
          setMissingAsk({ draft: final, missing: without, kind });
          return;
        }
        return api
          .checkDraft(final)
          .then(() => api.sendMessage(final))
          .then(onSent);
      })
      .catch((e: unknown) => {
        setError(toAppError(e));
        setBusy(null);
      });
  };

  /** Encrypt is on, but some recipients have no key: take them out, or send without encryption. */
  const answerMissing = (answer: 'remove' | 'plain' | 'cancel') => {
    const ask = missingAsk;
    setMissingAsk(null);
    if (!ask || answer === 'cancel') return;
    if (answer === 'plain') {
      touched.current.encrypt = true;
      const next = { ...ask.draft, encrypt: false };
      setDraft((d) => ({ ...d, encrypt: false }));
      sendNow(next);
      return;
    }
    const next = withoutRecipients(ask.draft, ask.missing);
    setDraft((d) => withoutRecipients(d, ask.missing));
    setFieldsRevision((r) => r + 1);
    sendNow(next);
  };

  /**
   * Check Names: look up what was typed in To, Cc and Bcc. Known names become
   * "Name <address>"; a name with several matches asks which; anything else
   * is marked. With `thenSend`, the message goes once everything is resolved.
   */
  const checkNames = (thenSend: boolean) => {
    setError(null);
    setStatus(null);
    const snapshot = draft;
    const fields = FIELDS.filter((f) => snapshot[f].trim() !== '');
    setBusy('check');
    Promise.all(fields.map((f) => api.resolveRecipients(snapshot[f]).then((r): [FieldName, RecipientCheck] => [f, r])))
      .then((results) => {
        setBusy(null);
        const patch: Partial<Draft> = {};
        const errors: Partial<Record<FieldName, string>> = {};
        const queue: Choice[] = [];
        for (const [f, check] of results) {
          patch[f] = [...check.resolved.map(entryFor), ...check.ambiguous.map((a) => a.input), ...check.invalid].join(', ');
          if (check.invalid.length > 0) {
            errors[f] = `Check ${check.invalid.join(', ')}. That does not look like an email address or the name of a contact.`;
          }
          queue.push(...check.ambiguous.map((a) => ({ field: f, input: a.input, candidates: a.candidates })));
        }
        const next = { ...snapshot, ...patch };
        setDraft((d) => ({ ...d, ...patch }));
        setFieldsRevision((r) => r + 1);
        setFieldErrors(errors);
        setChoices(queue);
        const clean = queue.length === 0 && Object.keys(errors).length === 0;
        setSendAfterChoices(thenSend && queue.length > 0 && Object.keys(errors).length === 0);
        if (thenSend && clean) sendNow(next);
        else if (!thenSend && clean) setStatus('Names checked.');
      })
      .catch((e: unknown) => {
        setBusy(null);
        setError(toAppError(e));
      });
  };

  const choose = (choice: Choice, person: Mailbox | null) => {
    const rest = choices.slice(1);
    setChoices(rest);
    if (person === null) {
      setSendAfterChoices(false);
      setFieldErrors((errors) => ({
        ...errors,
        [choice.field]: `Pick who ${choice.input} is, or type their address.`,
      }));
      return;
    }
    const entries = splitRecipients(draft[choice.field]).map((e) => (e === choice.input ? entryFor(person) : e));
    const next = { ...draft, [choice.field]: entries.join(', ') };
    setDraft(next);
    setFieldsRevision((r) => r + 1);
    if (rest.length === 0 && sendAfterChoices) {
      setSendAfterChoices(false);
      sendNow(next);
    }
  };

  const send = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    const empty = FIELDS.every((f) => parseRecipients(draft[f]).valid.length === 0 && parseRecipients(draft[f]).invalid.length === 0);
    // Names typed instead of addresses: look them up first.
    if (!empty && FIELDS.some((f) => parseRecipients(draft[f]).invalid.length > 0)) {
      checkNames(true);
      return;
    }
    sendNow(draft);
  };

  /** Save and remember the saved copy's id, so the next save or send replaces it. */
  const saveNow = (): Promise<void> => {
    const snapshot = draft;
    return api.saveDraft(snapshot).then(({ draftMessageId }) => {
      setDraft((d) => ({ ...d, draftMessageId }));
      setBaseline({ ...snapshot, draftMessageId });
      setUnsaved(false);
    });
  };

  const save = () => {
    setError(null);
    setBusy('save');
    saveNow().then(
      () => {
        setBusy(null);
        setStatus('Draft saved.');
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(null);
      },
    );
  };

  const attach = () => {
    setError(null);
    setBusy('attach');
    api.pickAttachments().then(
      (files) => {
        addFiles(files);
        setBusy(null);
      },
      (e: unknown) => {
        setError(toAppError(e));
        setBusy(null);
      },
    );
  };

  const popOut = () => {
    setError(null);
    setBusy('popout');
    api.openComposeWindow(draft).then(
      () => {
        setBusy(null);
        onPoppedOut?.();
      },
      (e: unknown) => {
        setBusy(null);
        setError(toAppError(e));
      },
    );
  };

  const requestClose = () => {
    if (dirty) setConfirmClose(true);
    else onClose();
  };

  // Going elsewhere in the app (a folder, Settings, another message) asks
  // the same question as closing, instead of dropping what was written.
  useEffect(() => {
    if (!onLeaveGuard || !dirty) return;
    onLeaveGuard((then) => {
      setAfterClose(() => then);
      setConfirmClose(true);
    });
    return () => {
      onLeaveGuard(null);
    };
  }, [onLeaveGuard, dirty]);
  const leave = afterClose ?? onClose;
  const keepEditing = () => {
    setConfirmClose(false);
    setAfterClose(null);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === 'Escape' && !confirmClose) {
      event.stopPropagation();
      requestClose();
    } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey) && busy === null && !loadingAttachments) {
      event.preventDefault();
      (event.currentTarget as HTMLFormElement).requestSubmit();
    } else if (event.altKey && !event.ctrlKey && !event.metaKey && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      if (busy === null) checkNames(false);
    } else if (event.key === 'Enter' && event.target instanceof HTMLInputElement) {
      // Plain Enter in To, Cc, Bcc or Subject must not send the message
      // (browsers submit a form on Enter in a text field).
      event.preventDefault();
    }
  };

  const field = (name: FieldName, label: string, autoFocus: boolean) => {
    const fieldId = `${id}-${name}`;
    const message = fieldErrors[name];
    return (
      <div className="compose__row">
        <label htmlFor={fieldId} className="compose__label">
          {label}
        </label>
        <div className="compose__field">
          <RecipientField
            key={fieldsRevision}
            id={fieldId}
            label={label}
            value={draft[name]}
            invalid={message !== undefined}
            unencryptable={noKeySet}
            describedBy={message ? `${fieldId}-error` : undefined}
            // The first field of a view the user just opened.
            // eslint-disable-next-line jsx-a11y/no-autofocus
            autoFocus={autoFocus && fieldsRevision === 0}
            onChange={(value) => {
              update({ [name]: value });
            }}
          />
          {message && (
            <p id={`${fieldId}-error`} className="compose__error" role="alert">
              {message}
            </p>
          )}
        </div>
        {name === 'to' && (!showCc || !showBcc) && (
          <span className="compose__copies">
            {!showCc && (
              <button
                type="button"
                className="btn btn--link"
                aria-label="Add Cc"
                onClick={() => {
                  setShowCc(true);
                }}
              >
                Cc
              </button>
            )}
            {!showBcc && (
              <button
                type="button"
                className="btn btn--link"
                aria-label="Add Bcc"
                onClick={() => {
                  setShowBcc(true);
                }}
              >
                Bcc
              </button>
            )}
          </span>
        )}
      </div>
    );
  };

  const choice = choices[0];
  const bodyHtml = htmlOf(draft);

  return (
    // The form handles Esc for its own fields; it is not an interactive control itself.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <form
      className={`compose${standalone ? ' compose--window' : ''}`}
      aria-labelledby={`${id}-title`}
      onSubmit={send}
      onKeyDown={onKeyDown}
      noValidate
    >
      <div className="compose__card">
      <header className="compose__head">
        <h1 id={`${id}-title`} className="compose__title">
          {title}
        </h1>
        {status && (
          <span className="muted small" role="status">
            {status}
          </span>
        )}
        {onPoppedOut && !phone && (
          <button
            type="button"
            className="icon-btn compose__popout"
            aria-label="Pop out"
            title="Carry on in a window of its own"
            disabled={busy !== null || loadingAttachments}
            onClick={popOut}
          >
            <Icon name="external" />
          </button>
        )}
        <button
          type="button"
          className={`icon-btn${onPoppedOut && !phone ? '' : ' compose__close'}`}
          aria-label="Close"
          onClick={requestClose}
        >
          <Icon name="close" />
        </button>
      </header>

      <div className="compose__fields">
        <div className="compose__row">
          <label htmlFor={`${id}-from`} className="compose__label">
            From
          </label>
          <select
            id={`${id}-from`}
            className="compose__from"
            value={draft.accountId}
            onChange={(e) => {
              update({ accountId: e.target.value });
            }}
          >
            {accounts.map((a) => (
              <option key={a.id} value={a.id}>
                {a.displayName && a.displayName !== a.email ? `${a.displayName} <${a.email}>` : a.email}
              </option>
            ))}
          </select>
        </div>
        {field('to', 'To', initial.to === '')}
        {showCc && field('cc', 'Cc', false)}
        {showBcc && field('bcc', 'Bcc', false)}
        {noKey.length > 0 && (
          <p className="compose__error compose__no-key" role="status">
            <Icon name="lock" size={14} />
            Can’t encrypt to {noKey.join(', ')}: there’s no {kindNoun(securityKind)} for {noKey.length === 1 ? 'them' : 'these recipients'}.
          </p>
        )}
        <div className="compose__row">
          <label htmlFor={`${id}-subject`} className="compose__label">
            Subject
          </label>
          <input
            id={`${id}-subject`}
            type="text"
            className="compose__input compose__input--subject"
            value={draft.subject}
            onChange={(e) => {
              update({ subject: e.target.value });
            }}
          />
        </div>
      </div>

      <div className="compose__tools">
        <button type="button" className="btn btn--ghost" disabled={busy !== null} onClick={attach}>
          <Icon name="paperclip" />
          Attach files
        </button>
        <MenuButton label="Insert" text="Insert" className="btn btn--ghost" align="left" disabled={busy !== null}>
          <MenuItem
            onSelect={() => {
              setInserting('symbol');
            }}
          >
            Symbol…
          </MenuItem>
          <MenuItem
            onSelect={() => {
              setInserting('contact');
            }}
          >
            Contact card…
          </MenuItem>
          <MenuItem
            onSelect={() => {
              setInserting('item');
            }}
          >
            Item…
          </MenuItem>
        </MenuButton>
        <button
          type="button"
          className="btn btn--ghost"
          title="Check names (Alt+K)"
          disabled={busy !== null}
          onClick={() => {
            checkNames(false);
          }}
        >
          <Icon name="people" />
          {busy === 'check' ? 'Checking' : 'Check names'}
        </button>
        <div className="compose__importance" role="group" aria-label="Importance">
          {IMPORTANCE.map(([value, text, title]) => (
            <button
              key={value}
              type="button"
              className={`compose__importance-btn compose__importance-btn--${value}`}
              aria-pressed={(draft.importance ?? 'normal') === value}
              title={title}
              onClick={() => {
                if ((draft.importance ?? 'normal') !== value) update({ importance: value });
              }}
            >
              {value === 'high' && <span aria-hidden="true">! </span>}
              {value === 'low' && <Icon name="arrowDown" size={14} />}
              {text}
            </button>
          ))}
        </div>
        <MenuButton
          label="Options"
          text="Options"
          className="btn btn--ghost"
          align="left"
          active={draft.requestReadReceipt === true || (hasKey && (draft.sign === true || wantEncrypt))}
        >
          <MenuItem
            kind="check"
            checked={draft.requestReadReceipt === true}
            onSelect={() => {
              update({ requestReadReceipt: draft.requestReadReceipt !== true });
            }}
          >
            Request a read receipt
          </MenuItem>
          <MenuSeparator />
          <MenuItem
            kind="check"
            checked={hasKey && draft.sign === true}
            disabled={!hasKey}
            onSelect={() => {
              touched.current.sign = true;
              update({ sign: draft.sign !== true });
            }}
          >
            Sign
            {!hasKey && <span className="menu__note">Needs a certificate or key for this account, in Settings › Security</span>}
          </MenuItem>
          <MenuItem
            kind="check"
            checked={hasKey && wantEncrypt}
            disabled={!hasKey}
            onSelect={() => {
              touched.current.encrypt = true;
              update({ encrypt: !wantEncrypt });
            }}
          >
            Encrypt
            {!hasKey && <span className="menu__note">Needs a certificate or key for this account, in Settings › Security</span>}
          </MenuItem>
          {bothKinds && (
            <MenuGroup label="Sign and encrypt with">
              {(['smime', 'openpgp'] as const).map((kind) => (
                <MenuItem
                  key={kind}
                  kind="radio"
                  checked={securityKind === kind}
                  onSelect={() => {
                    if (securityKind !== kind) update({ securityKind: kind });
                  }}
                >
                  {kindLabel(kind)}
                </MenuItem>
              ))}
            </MenuGroup>
          )}
        </MenuButton>
        {signatures && signatures.items.length > 0 && (
          <label className="compose__signature-pick">
            <span>Signature</span>
            <select
              className="field"
              value={signatureId ?? ''}
              onChange={(e) => {
                applySignature(signatures, e.target.value || null, false);
              }}
            >
              {signatures.items.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
              <option value="">None</option>
            </select>
          </label>
        )}
        <span className="compose__hint">Ctrl+Enter to send</span>
      </div>
      {draft.requestReadReceipt === true && (
        <p className="compose__option-note form-hint">A read receipt is requested. The recipient can choose not to send one.</p>
      )}
      {securityNote && (
        <p className="compose__option-note compose__security-note form-hint">
          <Icon name={wantEncrypt ? 'lock' : 'seal'} size={14} />
          {securityNote}
        </p>
      )}

      <RichEditorSlot
          label="Message"
          value={bodyHtml}
          // Replies start in the body, since To is already filled.
          focusOnOpen={initial.to !== ''}
          handleRef={editorRef}
          phone={phone}
          onChange={(html, text) => {
            setDraft((d) => withBody(d, html, text));
            setStatus(null);
          }}
          onNormalize={(from, to, text) => {
            const tidy = (d: Draft) => (htmlOf(d) === from ? withBody(d, to, text) : d);
            setDraft(tidy);
            setBaseline(tidy);
          }}
          fallback={<EditorLoading focus={initial.to !== ''} />}
        />

      {signatureImage && draft.bodyText.includes(signatureBlock(signatureImage.text)) && (
        <figure className="compose__signature">
          <img
            src={`data:${signatureImage.image.mimeType};base64,${signatureImage.image.data}`}
            alt="Your signature"
            style={{ width: Math.min(signatureImage.image.width, 240) }}
          />
          <figcaption className="form-hint">Your signature picture is added under your signature when sent.</figcaption>
        </figure>
      )}

      {loadingAttachments && (
        <p className="compose__loading form-hint" role="status" aria-busy="true">
          Adding the original attachments
        </p>
      )}
      {attachError && (
        <div className="notice notice--error compose__attach-error" role="alert">
          <p className="notice__title">Couldn’t add the original attachments</p>
          <p>{attachError.message} You can still send the message, or add files with Attach files.</p>
        </div>
      )}

      {draft.attachments.length > 0 && (
        <div className="compose__attachments">
          <ul className="attachments" aria-label="Attached files">
            {draft.attachments.map((file) => (
              <li key={file.path} className="attachment">
                <span className="attachment__text">
                  <span className="attachment__name">{file.fileName}</span>
                  <span className="muted small">{formatBytes(file.size)}</span>
                </span>
                <button
                  type="button"
                  className="icon-btn"
                  aria-label={`Remove ${file.fileName}`}
                  onClick={() => {
                    update({ attachments: draft.attachments.filter((a) => a.path !== file.path) });
                  }}
                >
                  <Icon name="close" />
                </button>
              </li>
            ))}
          </ul>
          <p className={overLimit ? 'form-error' : 'form-hint'}>
            Attachments {formatBytes(attachmentBytes)}
            {account?.sizeLimitBytes != null &&
              `. ${account.email} accepts messages up to ${formatBytes(account.sizeLimitBytes)} once encoded`}
            {overLimit && '. This is probably too large to send.'}
          </p>
        </div>
      )}

      {error && (
        <div className="notice notice--error" role="alert">
          <p className="notice__title">{errorTitle(error.kind)}</p>
          <p>{error.message}</p>
        </div>
      )}

      <footer className="compose__foot">
        <button type="submit" className="btn btn--primary btn--large" disabled={busy !== null || loadingAttachments}>
          <Icon name="sent" size={16} />
          {busy === 'send' ? 'Sending' : 'Send'}
        </button>
        <button type="button" className="btn btn--secondary" disabled={busy !== null} onClick={save}>
          {busy === 'save' ? 'Saving' : 'Save draft'}
        </button>
        {offlineNote && (
          <span className="compose__note">
            <Icon name="clock" size={16} />
            No connection right now. It waits in the Outbox and sends by itself.
          </span>
        )}
        <span className="statusbar__spacer" />
        <button type="button" className="icon-btn" aria-label="Discard" onClick={requestClose}>
          <Icon name="trash" />
        </button>
      </footer>
      </div>

      {inserting === 'symbol' && (
        <SymbolDialog
          onClose={() => {
            setInserting(null);
            editorRef.current?.focus();
          }}
          onPick={(symbol) => {
            setInserting(null);
            editorRef.current?.insertText(symbol);
          }}
        />
      )}
      {(inserting === 'contact' || inserting === 'item') &&
        (() => {
          const Picker = inserting === 'contact' ? ContactCardDialog : ItemDialog;
          return (
            <Picker
              onClose={() => {
                setInserting(null);
              }}
              onAttach={(file) => {
                setInserting(null);
                addFiles([file]);
                setStatus(`Attached ${file.fileName}.`);
              }}
            />
          );
        })()}

      {missingAsk && (
        <MissingKeysDialog
          missing={missingAsk.missing}
          kind={missingAsk.kind}
          everyone={recipientAddresses(missingAsk.draft).length <= missingAsk.missing.length}
          onRemove={() => {
            answerMissing('remove');
          }}
          onSendUnencrypted={() => {
            answerMissing('plain');
          }}
          onCancel={() => {
            answerMissing('cancel');
          }}
        />
      )}

      {choice && (
        <NameChooser
          key={`${choice.field}:${choice.input}`}
          input={choice.input}
          candidates={choice.candidates}
          onPick={(person) => {
            choose(choice, person);
          }}
          onCancel={() => {
            choose(choice, null);
          }}
        />
      )}

      {confirmClose && (
        <Modal
          title="Discard this message?"
          description={
            unsaved
              ? 'It is no longer in the Outbox and is not saved anywhere else.'
              : 'You have changes that are not saved.'
          }
          size="small"
          onClose={keepEditing}
        >
          <div className="modal__actions">
            <button type="button" className="btn btn--secondary" onClick={keepEditing}>
              Keep editing
            </button>
            <button
              type="button"
              className="btn btn--secondary"
              onClick={() => {
                setConfirmClose(false);
                saveNow().then(leave, (e: unknown) => {
                  setAfterClose(null);
                  setError(toAppError(e));
                });
              }}
            >
              Save draft
            </button>
            <button
              type="button"
              className="btn btn--danger"
              onClick={() => {
                keepEditing();
                leave();
              }}
            >
              Discard
            </button>
          </div>
        </Modal>
      )}
    </form>
  );
}
