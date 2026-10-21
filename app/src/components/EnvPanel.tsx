import { T } from '../i18n';
import type { JavaRuntime } from '../lib/types';

interface Props {
  java: JavaRuntime[] | null;
  loading: boolean;
  error: string | null;
  onRefresh: () => void;
  onOpenDir: (path: string) => void;
  gameDir: string;
}

/**
 * 本机环境：探测到的 Java 运行时和当前目录。
 * 装之前先看一眼有没有合适的 Java，比装到一半失败再回来查要省事。
 */
export function EnvPanel({ java, loading, error, onRefresh, onOpenDir, gameDir }: Props) {
  return (
    <>
      {loading && <span className="ef-skeleton line" aria-label={T.EnvPanel.loading} />}

      {error && (
        <p className="ef-notice" data-flag={T.EnvPanel.errorFlag}>
          {error}
        </p>
      )}

      {java && java.length === 0 && !loading && (
        <p className="ef-empty">
          {T.EnvPanel.noJava}
        </p>
      )}

      {java && java.length > 0 && (
        <ul className="ef-ledger">
          {java.map((rt) => (
            <li key={rt.path}>
              <span title={rt.path}>
                Java {rt.major} · {rt.vendor}
              </span>
              <span className="ef-field">
                {rt.version} · {rt.arch}
              </span>
            </li>
          ))}
        </ul>
      )}

      <ul className="ef-ledger">
        <li>
          <span>{T.common.gameDir}</span>
          <span className="ef-field" title={gameDir}>
            {gameDir || T.EnvPanel.unset}
          </span>
        </li>
      </ul>

      <p className="env-note">
        {T.EnvPanel.note}
      </p>

      <p className="ef-actions" style={{ marginTop: '0.9rem' }}>
        <button className="ef-btn small" type="button" onClick={onRefresh}>
          {T.EnvPanel.refresh}
        </button>
        <button
          className="ef-btn small"
          type="button"
          onClick={() => onOpenDir(gameDir)}
          disabled={!gameDir}
        >
          {T.common.openDir}
        </button>
      </p>
    </>
  );
}
