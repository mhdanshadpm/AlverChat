export type MenuPoint = { left:number; top:number };
export function placeContextMenu(x:number,y:number,viewportWidth:number,viewportHeight:number,width=220,height=396):MenuPoint{
  return {left:Math.max(8,Math.min(x,viewportWidth-width-8)),top:Math.max(8,Math.min(y,viewportHeight-height-8))};
}
