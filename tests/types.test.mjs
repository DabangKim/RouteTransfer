import {test} from 'node:test';
import assert from 'node:assert/strict';
import {parentPath,size} from '../src/types.ts';
test('Windows drive roots and POSIX roots stay valid',()=>{assert.equal(parentPath('C:\\photos\\team'),'C:\\photos');assert.equal(parentPath('C:\\photos'),'C:\\');assert.equal(parentPath('C:\\'),'C:\\');assert.equal(parentPath('/photos/team/'),'/photos');assert.equal(parentPath('/'),'/')});
test('large aggregate sizes are displayed without 32-bit truncation',()=>{assert.equal(size('107374182400'),'100.0 GiB');assert.equal(size(0),'0 B')});
