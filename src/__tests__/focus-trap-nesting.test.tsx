import {useState} from 'react';
import {fireEvent,render,screen,waitFor} from '@testing-library/react';
import {expect,it,vi} from 'vitest';
import {useFocusTrap} from '@/hooks/use-focus-trap';
it('isolates nested dialogs, closes only the top dialog, and restores focus and background state',async()=>{
 const parentClose=vi.fn();
 function Child({close}:{close:()=>void}){const ref=useFocusTrap<HTMLDivElement>({active:true,onEscape:close});return <div ref={ref} role="dialog" aria-label="Child"><input aria-label="Child field"/><button onClick={close}>Close child</button></div>;}
 function Parent(){const [child,setChild]=useState(false);const ref=useFocusTrap<HTMLDivElement>({active:true,onEscape:parentClose});return <div ref={ref} role="dialog" aria-label="Parent"><button onClick={()=>setChild(true)}>Open child</button>{child&&<Child close={()=>setChild(false)}/>}</div>;}
 const background=document.createElement('button');background.textContent='Background';document.body.append(background);
 const view=render(<Parent/>);
 try{
  await waitFor(()=>expect(screen.getByRole('button',{name:'Open child'})).toHaveFocus());
  expect(background.inert).toBe(true);
  fireEvent.click(screen.getByRole('button',{name:'Open child'}));
  await waitFor(()=>expect(screen.getByLabelText('Child field')).toHaveFocus());
  background.focus();expect(screen.getByLabelText('Child field')).toHaveFocus();
  fireEvent.keyDown(document,{key:'Escape'});
  expect(screen.queryByRole('dialog',{name:'Child'})).toBeNull();expect(parentClose).not.toHaveBeenCalled();
  expect(screen.getByRole('button',{name:'Open child'})).toHaveFocus();expect(background.inert).toBe(true);
  fireEvent.keyDown(document,{key:'Escape'});expect(parentClose).toHaveBeenCalledOnce();
  view.unmount();expect(background.inert).toBeFalsy();
 }finally{view.unmount();background.remove();}
});
