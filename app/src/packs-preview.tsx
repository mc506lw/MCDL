// 临时预览入口：把「整合包」页单独挂起来，验证完就删。
// 浏览器里没有 Tauri，所以 `packs-api.ts` 走的是它自带的演示后端。

import { useState } from 'react';
import ReactDOM from 'react-dom/client';

import { PacksPage } from './pages/PacksPage';
import './styles/endfield.css';
import './styles/app.css';

const DEFAULT_DIR = 'C:\\Users\\Player\\AppData\\Roaming\\.minecraft';

function Preview() {
  const [dir, setDir] = useState(DEFAULT_DIR);
  return (
    <main className="app-main">
      <PacksPage
        gameDir={dir}
        dirs={[DEFAULT_DIR]}
        onChangeDir={setDir}
        onOpenSettings={() => undefined}
      />
    </main>
  );
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(<Preview />);
