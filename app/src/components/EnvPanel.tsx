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
      {loading && <span className="ef-skeleton line" aria-label="正在探测本机 Java" />}

      {error && (
        <p className="ef-notice" data-flag="探测失败">
          {error}
        </p>
      )}

      {java && java.length === 0 && !loading && (
        <p className="ef-empty">
          本机没找到 Java。安装时会按版本需要从 Adoptium 取一份，放在游戏目录的 runtime 下。
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
          <span>游戏目录</span>
          <span className="ef-field" title={gameDir}>
            {gameDir || '未设置'}
          </span>
        </li>
      </ul>

      <p className="env-note">
        没有合适的 Java 时按版本自动取一份；官方 Adoptium 失败会改用清华镜像（第三方）取同一份
        Temurin 构建。
      </p>

      <p className="ef-actions" style={{ marginTop: '0.9rem' }}>
        <button className="ef-btn small" type="button" onClick={onRefresh}>
          重新探测 Java
        </button>
        <button
          className="ef-btn small"
          type="button"
          onClick={() => onOpenDir(gameDir)}
          disabled={!gameDir}
        >
          打开游戏目录
        </button>
      </p>
    </>
  );
}
