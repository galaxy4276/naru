import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { TooltipProvider } from '@/components/ui/tooltip';
import { App } from './App';
import { Boundary } from './ui/Boundary';
import './index.css';

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <TooltipProvider delayDuration={400}>
      <Boundary name="Naru 화면" className="m-6 rounded-xl bg-background p-4">
        <App />
      </Boundary>
    </TooltipProvider>
  </StrictMode>,
);
