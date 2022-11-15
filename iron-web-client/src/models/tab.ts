import {Guid} from "guid-typescript";

export class Tab{
  id: Guid;
  name?: string;

  constructor (id:Guid, name?:string){
    this.id = id;
    this.name = name;
  }
}
