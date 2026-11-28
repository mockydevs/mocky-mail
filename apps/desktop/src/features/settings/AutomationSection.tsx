import { useEffect, useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, AutomationInfo, Settings } from '../../api/types';
import { copyText } from '../../lib/clipboard';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog } from '../../ui/Modal';

export const AUTOMATION_DOCS = 'https://github.com/mockydevs/mocky-mail/blob/main/docs/automation.md';

type Mode = Settings['automation'];

const MODES: readonly { value: Mode; label: string; hint: string }[] = [
  { value: 'off', label: 'Off', hint: 'No program can use Mocky Mail. Nothing listens for them.' },
  {
    value: 'read',
    label: 'Look only',
    hint: 'Programs can list and read your mail, calendar and contacts, but can’t change anything.',
  },
  {
    value: 'full',
    label: 'Full access',
    hint: 'Programs can also send, move and delete mail, and change events, contacts and rules.',
  },
];

/** The info for the mode chosen; the backend may still be switching. */
const settled = (info: AutomationInfo | null, mode: Mode) => info !== null && info.mode === mode;

/**
 * Settings › Automation (computers only): whether the mocky-mail command line
 * and AI tools that speak MCP may use the app, and how to connect them.
 */
export function AutomationSection({
  settings,
  onChange,
}: {
  settings: Settings;
  onChange: (next: Settings) => void;
}) {
  const api = useMailApi();
  const id = useId();
  const mode = settings.automation;
  const [info, setInfo] = useState<AutomationInfo | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [confirmReset, setConfirmReset] = useState(false);
  const [resetting, setResetting] = useState(false);
  const [done, setDone] = useState<string | null>(null);

  // Ask again whenever the mode changes. The setting is saved a moment
  // before the channel opens, so an answer for the old mode is asked again.
  useEffect(() => {
    let live = true;
    let tries = 0;
    let timer: number | undefined;
    const load = () => {
      api.getAutomationInfo().then(
        (next) => {
          if (!live) return;
          setInfo(next);
          setError(null);
          if (next.mode !== mode && tries < 5) {
            tries += 1;
            timer = window.setTimeout(load, 300);
          }
        },
        (e: unknown) => {
          if (live) setError(toAppError(e));
        },
      );
    };
    load();
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [api, mode]);

  const copy = (what: string, text: string) => {
    void copyText(text).then((ok) => {
      setCopied(ok ? what : null);
      setDone(ok ? null : 'Couldn’t copy. Select the text and copy it yourself.');
    });
  };

  const reset = () => {
    setConfirmReset(false);
    setResetting(true);
    setDone(null);
    api.resetAutomationToken().then(
      (next) => {
        setInfo(next);
        setResetting(false);
        setDone('New secret made. Programs that used the old one need to be started again.');
      },
      (e: unknown) => {
        setResetting(false);
        setError(toAppError(e));
      },
    );
  };

  const shown = settled(info, mode) ? info : null;
  const copyButton = (what: string, text: string, label: string) => (
    <button
      type="button"
      className="btn btn--secondary btn--small"
      onClick={() => {
        copy(what, text);
      }}
    >
      <Icon name={copied === what ? 'check' : 'copy'} size={16} />
      {copied === what ? 'Copied' : label}
    </button>
  );

  return (
    <section className="settings__group settings__group--wide" aria-labelledby={`${id}-title`} data-section="automation">
      <h2 id={`${id}-title`} className="settings__legend">
        Automation
      </h2>
      <p className="form-hint">
        Let programs on this computer, like the mocky-mail command line or AI tools that speak MCP (for example Claude
        Desktop), work with your mail. Only programs running on this computer as you can connect.
      </p>

      <fieldset className="settings__sub">
        <legend className="form-field__label">Programs on this computer can</legend>
        {MODES.map((m) => (
          <label key={m.value} className="choice choice--described">
            <input
              type="radio"
              name={`${id}-mode`}
              checked={mode === m.value}
              aria-describedby={`${id}-${m.value}-hint`}
              onChange={() => {
                setDone(null);
                setCopied(null);
                onChange({ ...settings, automation: m.value });
              }}
            />
            <span>
              {m.label}
              <span id={`${id}-${m.value}-hint`} className="choice__hint">
                {m.hint}
              </span>
            </span>
          </label>
        ))}
      </fieldset>

      {mode === 'full' && (
        <div className="notice notice--warn" role="note">
          <p className="notice__title">Full access lets programs send, move and delete mail</p>
          <p>
            A program, or an AI tool following instructions hidden in an email, could send messages as you or delete
            them. Choose this only for tools you trust, and prefer Look only when that is enough.
          </p>
        </div>
      )}

      {mode !== 'off' && !shown && !error && (
        <p className="form-hint" role="status">
          Turning it on…
        </p>
      )}

      {mode !== 'off' && shown && (
        <dl className="automation">
          <div className="automation__row">
            <dt>Command line</dt>
            <dd>
              {shown.cliPath ? (
                <span className="automation__value">
                  <code>{shown.cliPath}</code>
                  {copyButton('cli', shown.cliPath, 'Copy path')}
                </span>
              ) : (
                <span className="muted">
                  The mocky-mail program wasn’t found next to Mocky Mail. The guide below says how to get it.
                </span>
              )}
            </dd>
          </div>
          {shown.mcpConfig && (
            <div className="automation__row">
              <dt>
                <label htmlFor={`${id}-mcp`}>MCP configuration</label>
              </dt>
              <dd>
                <textarea
                  id={`${id}-mcp`}
                  className="field code-box"
                  readOnly
                  spellCheck={false}
                  rows={Math.min(12, shown.mcpConfig.split('\n').length)}
                  value={shown.mcpConfig}
                />
                <span className="automation__note">
                  Paste this into your AI tool’s MCP settings (in Claude Desktop: Settings › Developer › Edit Config).
                </span>
                {copyButton('mcp', shown.mcpConfig, 'Copy')}
              </dd>
            </div>
          )}
          <div className="automation__row">
            <dt>Secret</dt>
            <dd>
              {shown.tokenFile ? (
                <span className="automation__note">
                  Programs read it from <code>{shown.tokenFile}</code>, a file only you can open.
                </span>
              ) : null}
              <button
                type="button"
                className="btn btn--secondary btn--small"
                disabled={resetting}
                aria-busy={resetting || undefined}
                onClick={() => {
                  setConfirmReset(true);
                }}
              >
                <Icon name="key" size={16} />
                {resetting ? 'Resetting…' : 'Reset secret'}
              </button>
            </dd>
          </div>
        </dl>
      )}
      {mode !== 'off' && shown?.endpoint === null && (
        <p className="form-hint" role="status">
          Not listening yet. If this doesn’t change, another program may be using the same name; restart Mocky Mail.
        </p>
      )}

      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {done && (
        <p className="form-hint" role="status">
          {done}
        </p>
      )}

      <p>
        <button
          type="button"
          className="btn btn--link"
          onClick={() => {
            api.openExternal(AUTOMATION_DOCS).catch((e: unknown) => {
              setError(toAppError(e));
            });
          }}
        >
          <Icon name="external" size={16} />
          How to connect the command line and AI tools
        </button>
      </p>

      {confirmReset && (
        <ConfirmDialog
          title="Reset the secret?"
          description="Mocky Mail makes a new secret. Programs using the old one stop working until they are started again."
          confirmLabel="Reset secret"
          danger
          onCancel={() => {
            setConfirmReset(false);
          }}
          onConfirm={reset}
        />
      )}
    </section>
  );
}
