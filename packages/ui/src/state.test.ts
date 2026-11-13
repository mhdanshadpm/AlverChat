import { describe, expect, it } from 'vitest';
import { dockPriority, shouldShowBubble, visibleBubbles } from './state';

describe('floating dock behavior',()=>{
  it('keeps only the three newest message bubbles',()=>{expect(visibleBubbles([1,2,3,4].map(id=>({id:String(id),sender:'peer',text:'hi',sentAt:id}))).map(b=>b.id)).toEqual(['2','3','4'])});
  it('prioritizes a call over unread and idle states',()=>{expect(dockPriority(true,3)).toBe('call');expect(dockPriority(false,2)).toBe('unread');expect(dockPriority(false,0)).toBe('idle')});
  it('suppresses bubbles for open, muted, and DND conversations',()=>{expect(shouldShowBubble(false,false,false)).toBe(true);expect(shouldShowBubble(true,false,false)).toBe(false);expect(shouldShowBubble(false,true,false)).toBe(false);expect(shouldShowBubble(false,false,true)).toBe(false)});
});
