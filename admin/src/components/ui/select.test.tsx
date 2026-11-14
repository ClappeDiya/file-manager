// @vitest-environment jsdom
import '@testing-library/jest-dom/vitest';
import { expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { Select } from './select';
it('gives repeated field labels unique associations and announces validation errors', () => {
  render(<><Select label="Role" options={[{value:'a',label:'Auditor'}]} error="Choose an authorized role" /><Select label="Role" options={[{value:'b',label:'Owner'}]} /></>);
  const fields=screen.getAllByRole('combobox',{name:'Role'});
  expect(fields[0].id).not.toBe(fields[1].id);
  expect(fields[0]).toHaveAttribute('aria-invalid','true');
  expect(fields[0]).toHaveAccessibleDescription('Choose an authorized role');
  expect(screen.getByRole('alert')).toHaveTextContent('Choose an authorized role');
});
