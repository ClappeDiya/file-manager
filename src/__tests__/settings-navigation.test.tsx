import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {it,expect,beforeEach} from 'vitest';
import {SimpleModeWrapper} from '@/components/simple-mode/simple-mode-wrapper';
import {useUIStore} from '@/stores/ui-store';
beforeEach(()=>useUIStore.setState({appMode:'simple',sidebarOpen:true}));
it('reaches real console enrollment from the labeled Settings navigation in simple mode',async()=>{
 render(<SimpleModeWrapper fileManager={<div>Files workspace</div>}/>);
 fireEvent.click(screen.getByRole('button',{name:'Settings'}));
 fireEvent.click(screen.getByRole('button',{name:'All settings'}));
 const consoleSection=screen.getByRole('button',{name:/Managed by Console/});
 expect(consoleSection).toHaveAttribute('aria-expanded','false');
 fireEvent.click(consoleSection);
 await waitFor(()=>expect(screen.getByLabelText('Console URL')).toBeInTheDocument());
 expect(consoleSection).toHaveAttribute('aria-expanded','true');
 fireEvent.click(screen.getByRole('button',{name:'Back to settings overview'}));
 expect(screen.getByRole('button',{name:'All settings'})).toHaveFocus();
 fireEvent.keyDown(window,{key:',',metaKey:true});
 expect(screen.getByRole('button',{name:/Managed by Console/})).toBeInTheDocument();
});
