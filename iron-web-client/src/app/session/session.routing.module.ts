import { NgModule } from '@angular/core';
import { Routes, RouterModule } from '@angular/router';
import {SessionComponent} from "./session.component";

const routes: Routes = [
  {
    path: '',
    component: SessionComponent
  }
];

@NgModule({
  imports: [RouterModule.forChild(routes)],
  exports: [RouterModule]
})
export class SessionRoutingModule { }
