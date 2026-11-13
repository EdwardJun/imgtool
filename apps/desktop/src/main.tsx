import React from 'react';
import ReactDOM from 'react-dom/client';
import { App as AntApp, ConfigProvider, theme } from 'antd';
import zhCN from 'antd/locale/zh_CN';
import { isTauri } from '@tauri-apps/api/core';
import App from './App';
import './styles.css';

async function bootstrap() {
  // 开发时直接用浏览器打开 http://localhost:1420 会进入 Mock 模式（生产包不包含）
  // 注意：@tauri-apps/api 在导入时可能创建空的 __TAURI_INTERNALS__，所以用 isTauri 判断
  if (import.meta.env.DEV && !isTauri()) {
    const { install } = await import('./dev/browserMock');
    await install();
  }

  ReactDOM.createRoot(document.getElementById('root')!).render(
    <React.StrictMode>
      <ConfigProvider
        locale={zhCN}
        theme={{
          algorithm: theme.darkAlgorithm,
          token: {
            colorPrimary: '#4f5bff',
            colorBgBase: '#1f1f22',
            colorBgContainer: '#2b2b30',
            colorBgElevated: '#33333a',
            borderRadius: 6,
            fontSize: 13,
          },
        }}
      >
        <AntApp className="ant-app-root">
          <App />
        </AntApp>
      </ConfigProvider>
    </React.StrictMode>,
  );
}

bootstrap();
