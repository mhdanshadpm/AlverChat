import { describe, expect, it } from 'vitest';
import { placeContextMenu } from './menu';

describe('message context menu placement',()=>{
  it('flips the menu left and up when it would cross the viewport edge',()=>{expect(placeContextMenu(900,700,1024,768)).toEqual({left:796,top:364})});
  it('keeps the menu inside a narrow viewport',()=>{expect(placeContextMenu(370,650,400,680,220,300)).toEqual({left:172,top:372})});
});
