import {Component, Inject, OnInit, QueryList} from '@angular/core';
import {MatTab, MatTabGroup} from "@angular/material/tabs";
import {SessionService} from "../services/session.service";
import {Tab} from "../../models/tab";
import {Guid} from "guid-typescript";


@Component({
  selector: 'app-tabs',
  templateUrl: './tabs.component.html',
  styleUrls: ['./tabs.component.scss']
})
export class TabsComponent implements OnInit {
  public tabGroup: MatTabGroup;
  public tabNodes: QueryList<MatTab>;
  public tabs : Array<Tab>;

  constructor(public sessionService: SessionService) {
  }

  ngOnInit(): void {
    this.sessionService.tabs$.subscribe(tabs => this.tabs = tabs);
  }

  closeSession(id: Guid){
    this.sessionService.closeSession(id);
  }

  setCurrentSession(id:Guid){
    this.sessionService.setCurrentSessionById(id);
  }
}
